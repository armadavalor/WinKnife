<!--
  The WinKnife logo as vector art (a redraw of assets/logo.png), so the blade
  can move: it folds shut and opens again on hover of the nearest `.group`
  ancestor, and loops while `loading`. Both are CSS animations, so the
  animations switch (`data-animations="off"`) leaves the knife open and still.
-->
<script lang="ts">
  let {
    size = 32,
    loading = false,
    class: className = "",
  }: { size?: number; loading?: boolean; class?: string } = $props();
</script>

<svg
  viewBox="0 0 1024 1024"
  width={size}
  height={size}
  class="knife-logo shrink-0 {className}"
  class:is-loading={loading}
  aria-hidden="true"
  focusable="false"
>
  <circle cx="512" cy="512" r="512" fill="#0090e3" />
  <!-- Blade first, so it folds away behind the handle. -->
  <g class="knife-blade">
    <g transform="translate(645 422) rotate(-139.5)">
      <path
        d="M-30 -62 L250 -62 C350 -62 420 0 400 62 L-30 62 Z"
        fill="#bdbab9"
        stroke="#0b4d73"
        stroke-width="26"
        stroke-linejoin="round"
      />
      <path
        d="M0 -42 L248 -42 C330 -42 385 -6 380 40"
        fill="none"
        stroke="#ffffff"
        stroke-width="18"
        stroke-linecap="round"
      />
    </g>
  </g>
  <g transform="translate(767 431) rotate(39.7)">
    <path
      d="M-93 0 H93 V455 a70 70 0 0 1 -70 70 H-23 a70 70 0 0 1 -70 -70 Z"
      fill="#0c6499"
      stroke="#0b4d73"
      stroke-width="26"
      stroke-linejoin="round"
    />
    <g fill="#0b4d73" transform="translate(-36 380)">
      <rect width="34" height="34" />
      <rect x="40" width="34" height="34" />
      <rect y="40" width="34" height="34" />
      <rect x="40" y="40" width="34" height="34" />
    </g>
  </g>
</svg>

<style>
  .knife-blade {
    transform-box: view-box;
    transform-origin: 645px 422px;
  }

  /* Hover: one close-and-open, from whichever `.group` holds the logo. */
  :global(.group:hover) .knife-blade {
    animation: knife-fold 0.7s ease-in-out;
  }

  /* Loading: keep folding and opening until the work is done. */
  .is-loading .knife-blade {
    animation: knife-fold 1.4s ease-in-out infinite;
  }

  @keyframes knife-fold {
    0%,
    100% {
      transform: rotate(0deg);
    }
    50% {
      transform: rotate(-88deg);
    }
  }
</style>
