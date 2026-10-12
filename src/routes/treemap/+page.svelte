<!--
  Disk map: where the space actually went.

  Canvas rather than SVG, because a folder can easily produce four hundred
  rectangles and four hundred DOM nodes that re-layout on every hover is how a
  treemap ends up slower than the scan that built it (module spec 04).

  Two honesty rules the drawing has to keep:

  - A rectangle whose size is not known — a folder the scan could not read, or
    one it never covered — is hatched and labelled, never drawn as if it were
    empty. A silent zero is a lie about where space went.
  - A junction is drawn with a hollow outline and no size, because its bytes
    live somewhere else on the map and counting them twice would make the
    picture add up to more than the disk.
-->
<script lang="ts">
  import { goto } from "$app/navigation";
  import { onMount, untrack } from "svelte";

  import { revealInExplorer } from "$lib/api";
  import ProgressBar from "$lib/components/ProgressBar.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { formatBytes, formatDateTime, truncatePath } from "$lib/format";
  import { t } from "$lib/i18n";
  import { treemapStore } from "$lib/stores/treemap.svelte";
  import { hitTest, squarify, type Placed } from "$lib/treemap/squarify";
  import type { TreeChild, TreemapCategory } from "$lib/types";

  const store = treemapStore;

  /** Category colours. Muted on purpose — this is a map, not an alarm. */
  const COLOURS: Record<TreemapCategory, string> = {
    directory: "#4b5c74",
    video: "#7d5ba6",
    image: "#3f8f7a",
    audio: "#b5843a",
    archive: "#8a6d3b",
    code: "#3f6f9f",
    app: "#8f5148",
    document: "#5a7d55",
    other: "#5b6270",
  };

  let canvas = $state<HTMLCanvasElement | null>(null);
  let box = $state<HTMLDivElement | null>(null);
  let width = $state(800);
  let height = $state(520);
  let hovered = $state<TreeChild | null>(null);
  let menu = $state<{ x: number; y: number; child: TreeChild } | null>(null);
  let focused = $state(0);
  let placed: Placed<TreeChild>[] = [];

  // Run-once mounting, deliberately not reactive: as an effect this subscribed
  // to the store's own `status`, which is why the page sat on "Looking for
  // drives…" while the drive list was already loaded.
  onMount(() => {
    void store.loadDrives();
  });

  $effect(() => {
    // `untrack` so nothing the subscription touches on the way in can become a
    // dependency of this effect; the teardown is the only reason it is one.
    const listening = untrack(() => store.listen());
    return () => {
      void listening.then((unlisten) => unlisten());
    };
  });

  // Keep the canvas the size of its box, at device pixel density.
  $effect(() => {
    if (!box) {
      return;
    }
    const element = box;
    const observer = new ResizeObserver(() => {
      width = Math.max(320, Math.floor(element.clientWidth));
      height = Math.max(240, Math.floor(element.clientHeight));
    });
    observer.observe(element);
    return () => observer.disconnect();
  });

  // Repaint whenever the level, the size or the selection changes.
  $effect(() => {
    const level = store.level;
    const w = width;
    const h = height;
    const active = focused;
    if (!canvas) {
      return;
    }
    paint(canvas, level?.children ?? [], w, h, active);
  });

  function paint(
    target: HTMLCanvasElement,
    children: readonly TreeChild[],
    w: number,
    h: number,
    active: number,
  ): void {
    const dpr = window.devicePixelRatio || 1;
    target.width = Math.floor(w * dpr);
    target.height = Math.floor(h * dpr);
    target.style.width = `${w}px`;
    target.style.height = `${h}px`;

    const ctx = target.getContext("2d");
    if (!ctx) {
      return;
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    placed = squarify(children, (child) => child.sizeBytes, {
      x: 0,
      y: 0,
      w,
      h,
    });

    for (let i = 0; i < placed.length; i += 1) {
      drawCell(ctx, placed[i], i === active);
    }
  }

  function drawCell(
    ctx: CanvasRenderingContext2D,
    cell: Placed<TreeChild>,
    isFocused: boolean,
  ): void {
    const { x, y, w, h, item } = cell;
    if (w < 1 || h < 1) {
      return;
    }

    if (item.isLink) {
      // Hollow: it exists, it is not counted, and it is not empty either.
      ctx.strokeStyle = "#6b7280";
      ctx.setLineDash([4, 3]);
      ctx.strokeRect(x + 1, y + 1, w - 2, h - 2);
      ctx.setLineDash([]);
    } else {
      ctx.fillStyle = COLOURS[item.category] ?? COLOURS.other;
      ctx.fillRect(x, y, w, h);
      if (!item.sized) {
        // Hatching says "this number is a floor, not a fact".
        ctx.save();
        ctx.beginPath();
        ctx.rect(x, y, w, h);
        ctx.clip();
        ctx.strokeStyle = "rgba(255,255,255,0.28)";
        ctx.lineWidth = 1;
        for (let offset = -h; offset < w; offset += 7) {
          ctx.beginPath();
          ctx.moveTo(x + offset, y + h);
          ctx.lineTo(x + offset + h, y);
          ctx.stroke();
        }
        ctx.restore();
      }
      ctx.strokeStyle = "rgba(0,0,0,0.35)";
      ctx.lineWidth = 1;
      ctx.strokeRect(x + 0.5, y + 0.5, w - 1, h - 1);
    }

    if (isFocused) {
      ctx.strokeStyle = "#e8eaf0";
      ctx.lineWidth = 2;
      ctx.strokeRect(x + 1, y + 1, w - 2, h - 2);
    }

    // Labels only where one fits; a clipped label is worse than none.
    if (w > 64 && h > 22) {
      ctx.fillStyle = "rgba(255,255,255,0.94)";
      ctx.font = "12px ui-sans-serif, system-ui, sans-serif";
      ctx.textBaseline = "top";
      ctx.save();
      ctx.beginPath();
      ctx.rect(x + 4, y + 3, w - 8, h - 6);
      ctx.clip();
      ctx.fillText(item.name, x + 6, y + 5);
      if (h > 38) {
        ctx.fillStyle = "rgba(255,255,255,0.7)";
        ctx.fillText(
          item.sized ? formatBytes(item.sizeBytes) : t("treemap.sizeUnknown"),
          x + 6,
          y + 21,
        );
      }
      ctx.restore();
    }
  }

  function cellAt(event: MouseEvent): TreeChild | null {
    if (!canvas) {
      return null;
    }
    const rect = canvas.getBoundingClientRect();
    return (
      hitTest(placed, event.clientX - rect.left, event.clientY - rect.top)
        ?.item ?? null
    );
  }

  function onmove(event: MouseEvent): void {
    hovered = cellAt(event);
  }

  function onleave(): void {
    hovered = null;
  }

  function onclick(event: MouseEvent): void {
    menu = null;
    const child = cellAt(event);
    if (child && child.isDir && !child.isLink && child.path !== "") {
      void store.open(child.path);
      focused = 0;
    }
  }

  function oncontextmenu(event: MouseEvent): void {
    event.preventDefault();
    const child = cellAt(event);
    if (!child || child.path === "") {
      menu = null;
      return;
    }
    const rect = canvas?.getBoundingClientRect();
    menu = {
      x: event.clientX - (rect?.left ?? 0),
      y: event.clientY - (rect?.top ?? 0),
      child,
    };
  }

  /** Arrow keys walk the rectangles; Enter zooms. */
  function onkeydown(event: KeyboardEvent): void {
    const count = store.level?.children.length ?? 0;
    if (count === 0) {
      return;
    }
    if (event.key === "ArrowRight" || event.key === "ArrowDown") {
      event.preventDefault();
      focused = (focused + 1) % count;
    } else if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
      event.preventDefault();
      focused = (focused - 1 + count) % count;
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      const child = store.level?.children[focused];
      if (child?.isDir && !child.isLink && child.path !== "") {
        void store.open(child.path);
        focused = 0;
      }
    } else if (event.key === "Backspace") {
      event.preventDefault();
      const crumbs = store.level?.breadcrumb ?? [];
      if (crumbs.length > 1) {
        void store.open(crumbs[crumbs.length - 2]);
        focused = 0;
      }
    } else if (event.key === "Escape") {
      menu = null;
    }
  }

  function crumbLabel(path: string, index: number): string {
    if (index === 0) {
      return path;
    }
    const parts = path.split("\\").filter(Boolean);
    return parts[parts.length - 1] ?? path;
  }

  async function reveal(path: string): Promise<void> {
    menu = null;
    try {
      await revealInExplorer(path);
    } catch (error: unknown) {
      store.error =
        error && typeof error === "object" && "message" in error
          ? String((error as { message: unknown }).message)
          : String(error);
    }
  }

  function scanWithCleaner(path: string): void {
    menu = null;
    void goto(`/cleaner?from=${encodeURIComponent(path)}`);
  }

  function findDuplicatesHere(path: string): void {
    menu = null;
    void goto(`/duplicates?root=${encodeURIComponent(path)}`);
  }

  const usedPct = $derived.by(() => {
    const drive = store.drive;
    if (!drive || drive.totalBytes === 0) {
      return 0;
    }
    return Math.round(
      ((drive.totalBytes - drive.freeBytes) / drive.totalBytes) * 100,
    );
  });
</script>

<div class="mx-auto flex h-full max-w-6xl flex-col gap-5 px-8 py-8">
  <PageHeader title={t("treemap.title")} info={t("treemap.intro")} />

  {#if store.status === "loading" || store.status === "idle"}
    <p class="text-sm text-ink-faint">{t("treemap.loadingDrives")}</p>
  {:else if store.status === "error"}
    <div
      class="flex flex-col items-start gap-3 rounded-xl border border-surface-3 bg-surface-1 p-5"
    >
      <p class="text-sm">{t("treemap.drivesError")}</p>
      {#if store.error}
        <p class="font-mono text-xs text-ink-faint">{store.error}</p>
      {/if}
      <button
        type="button"
        class="rounded-lg bg-surface-3 px-3 py-1.5 text-sm transition-colors hover:bg-accent-soft"
        onclick={() => store.loadDrives()}
      >
        {t("dashboard.retry")}
      </button>
    </div>
  {:else if store.drives.length === 0}
    <EmptyState message={t("treemap.noDrives")} />
  {:else}
    <!-- Drive selector + capacity bar -->
    <section class="flex flex-wrap items-center gap-3">
      <label class="flex items-center gap-2 text-sm">
        <span class="text-ink-muted">{t("treemap.drive")}</span>
        <select
          class="rounded-lg border border-surface-3 bg-surface-1 px-2 py-1.5 text-sm"
          value={store.selected ?? ""}
          onchange={(event) =>
            store.select((event.currentTarget as HTMLSelectElement).value)}
        >
          {#each store.drives as drive (drive.root)}
            <option value={drive.root}>
              {drive.root}{drive.label ? ` — ${drive.label}` : ""}
            </option>
          {/each}
        </select>
      </label>

      {#if store.drive}
        <div class="flex min-w-56 flex-1 flex-col gap-1">
          <ProgressBar pct={usedPct} label={t("treemap.capacityLabel")} />
          <p class="text-xs text-ink-faint">
            {t("treemap.capacity", {
              used: formatBytes(store.drive.totalBytes - store.drive.freeBytes),
              total: formatBytes(store.drive.totalBytes),
              free: formatBytes(store.drive.freeBytes),
            })}
          </p>
        </div>
      {/if}

      {#if store.scanning}
        <button
          type="button"
          class="rounded-lg border border-surface-3 px-3 py-1.5 text-sm text-ink-muted transition-colors hover:bg-surface-2 hover:text-ink"
          onclick={() => store.stop()}
        >
          {t("treemap.stop")}
        </button>
      {:else}
        <button
          type="button"
          class="rounded-lg bg-accent-soft px-3 py-1.5 text-sm font-medium transition-colors hover:bg-accent hover:text-surface-0"
          onclick={() => store.scan()}
        >
          {store.cached ? t("treemap.rescan") : t("treemap.scan")}
        </button>
      {/if}
    </section>

    {#if store.scanning}
      <p class="text-sm text-ink-muted">
        {t("treemap.scanning", { count: store.scanned })}
        {#if store.currentPath}
          <span class="ml-2 font-mono text-xs text-ink-faint">
            {truncatePath(store.currentPath, 60)}
          </span>
        {/if}
      </p>
    {/if}

    {#if store.error}
      <p class="rounded-lg border border-risk-danger/30 p-3 text-sm text-risk-danger">
        {store.error}
      </p>
    {/if}

    {#if store.cached}
      <p class="text-xs text-ink-faint">
        {t("treemap.staleness", {
          when: formatDateTime(store.cached.scannedAt),
          size: formatBytes(store.cached.totalBytes),
          files: store.cached.fileCount,
        })}
        {#if !store.cached.complete}
          · {t("treemap.partial")}
        {/if}
        {#if store.cached.unreadableDirs > 0}
          · {t("treemap.unreadable", { count: store.cached.unreadableDirs })}
        {/if}
      </p>
    {/if}

    <!-- Breadcrumb -->
    {#if store.level}
      <nav class="flex flex-wrap items-center gap-1 text-sm" aria-label={t("treemap.breadcrumb")}>
        {#each store.level.breadcrumb as crumb, index (crumb)}
          {#if index > 0}
            <span class="text-ink-faint">/</span>
          {/if}
          <button
            type="button"
            class="rounded px-1.5 py-0.5 transition-colors hover:bg-surface-2 {index ===
            (store.level?.breadcrumb.length ?? 0) - 1
              ? 'text-ink'
              : 'text-ink-muted'}"
            onclick={() => store.open(crumb)}
          >
            {crumbLabel(crumb, index)}
          </button>
        {/each}
      </nav>
    {/if}

    <!-- The map -->
    <div bind:this={box} class="relative min-h-72 flex-1">
      {#if !store.level}
        <div class="flex h-full min-h-72 flex-col justify-center">
          <EmptyState message={t("treemap.empty")}>
            {#snippet action()}
              <button
                type="button"
                class="rounded-lg bg-accent-soft px-3 py-1.5 text-sm font-medium transition-colors hover:bg-accent hover:text-surface-0"
                onclick={() => store.scan()}
              >
                {t("treemap.scan")}
              </button>
            {/snippet}
          </EmptyState>
        </div>
      {:else if store.level.children.length === 0}
        <div
          class="flex h-full min-h-72 items-center justify-center rounded-xl border border-dashed border-surface-3 p-8"
        >
          <p class="text-sm text-ink-faint">
            {store.level.readable
              ? t("treemap.folderEmpty")
              : t("treemap.folderUnreadable")}
          </p>
        </div>
      {:else}
        <!--
          The canvas is the whole control, so it is the thing that takes focus
          and handles keys. `aria-label` describes the shortcuts, and the
          breadcrumb above stays a real list of buttons — a screen reader can
          navigate the tree there without the map.
        -->
        <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
        <canvas
          bind:this={canvas}
          class="block rounded-xl outline-none ring-offset-2 focus-visible:ring-2 focus-visible:ring-accent"
          tabindex="0"
          aria-label={t("treemap.canvasLabel")}
          onmousemove={onmove}
          onmouseleave={onleave}
          onclick={onclick}
          oncontextmenu={oncontextmenu}
          {onkeydown}
        ></canvas>

        {#if store.zooming}
          <p class="absolute left-3 top-3 rounded bg-surface-1/90 px-2 py-1 text-xs text-ink-muted">
            {t("treemap.opening")}
          </p>
        {/if}

        {#if hovered}
          <div
            class="pointer-events-none absolute bottom-3 left-3 max-w-[80%] rounded-lg bg-surface-1/95 px-3 py-2 text-xs shadow-lg"
          >
            <p class="font-mono text-ink">{truncatePath(hovered.path || hovered.name, 70)}</p>
            <p class="text-ink-muted">
              {#if hovered.isLink}
                {t("treemap.linkNote")}
              {:else if hovered.sized}
                {formatBytes(hovered.sizeBytes)}
              {:else}
                {t("treemap.sizeUnknownLong")}
              {/if}
              {#if hovered.itemCount > 1}
                · {t("treemap.foldedNote", { count: hovered.itemCount })}
              {/if}
            </p>
          </div>
        {/if}

        {#if menu}
          <div
            class="absolute z-20 w-56 rounded-lg border border-surface-3 bg-surface-1 p-1 shadow-2xl"
            style="left: {Math.min(menu.x, width - 232)}px; top: {Math.min(
              menu.y,
              height - 128,
            )}px"
          >
            <p class="truncate px-2 py-1 font-mono text-[0.65rem] text-ink-faint">
              {truncatePath(menu.child.path, 40)}
            </p>
            <button
              type="button"
              class="w-full rounded px-2 py-1.5 text-left text-xs transition-colors hover:bg-surface-2"
              onclick={() => reveal(menu?.child.path ?? "")}
            >
              {t("treemap.openInExplorer")}
            </button>
            <button
              type="button"
              class="w-full rounded px-2 py-1.5 text-left text-xs transition-colors hover:bg-surface-2"
              onclick={() => scanWithCleaner(menu?.child.path ?? "")}
            >
              {t("treemap.scanWithCleaner")}
            </button>
            <button
              type="button"
              class="w-full rounded px-2 py-1.5 text-left text-xs transition-colors hover:bg-surface-2"
              onclick={() =>
                findDuplicatesHere(
                  menu?.child.isDir
                    ? (menu?.child.path ?? "")
                    : (store.level?.path ?? ""),
                )}
            >
              {t("treemap.findDuplicates")}
            </button>
            <button
              type="button"
              class="w-full rounded px-2 py-1.5 text-left text-xs text-ink-muted transition-colors hover:bg-surface-2"
              onclick={() => (menu = null)}
            >
              {t("action.close")}
            </button>
          </div>
        {/if}
      {/if}
    </div>

    <!-- Legend and the notes the picture cannot draw -->
    <footer class="flex flex-wrap items-center gap-3 text-[0.7rem] text-ink-faint">
      {#each Object.entries(COLOURS) as [name, colour] (name)}
        <span class="flex items-center gap-1.5">
          <span class="inline-block size-2.5 rounded-sm" style="background:{colour}"></span>
          {t(`treemap.category.${name}` as never)}
        </span>
      {/each}
      {#if store.level && store.level.compressedBelow > 0}
        <span class="ml-auto">
          {t("treemap.compressedNote", { count: store.level.compressedBelow })}
        </span>
      {/if}
    </footer>
    {#if store.links.length > 0}
      <p class="text-[0.7rem] text-ink-faint">
        {t("treemap.linksNote", { count: store.links.length })}
      </p>
    {/if}
  {/if}
</div>
