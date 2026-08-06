<!--
  The three risk buckets, rendered the same way everywhere.

  Deliberately muted: ARCHITECTURE section 7 asks for "calm, minimal, no
  scare-ware red alerts". The red badge is informational — it means "we left
  this alone", not "danger".
-->
<script lang="ts">
  import { t } from "$lib/i18n";
  import type { RiskBucket } from "$lib/types";

  interface Props {
    /** Which bucket to show. */
    risk: RiskBucket;
    /** Optional item count shown after the label. */
    count?: number;
  }

  let { risk, count }: Props = $props();

  const labels: Record<RiskBucket, string> = {
    safe: t("risk.safe"),
    needsReview: t("risk.needsReview"),
    doNotDelete: t("risk.doNotDelete"),
  };

  const styles: Record<RiskBucket, string> = {
    safe: "border-risk-safe/40 text-risk-safe",
    needsReview: "border-risk-review/40 text-risk-review",
    doNotDelete: "border-risk-danger/40 text-risk-danger",
  };
</script>

<span
  class="inline-flex items-center gap-1.5 rounded-md border px-2 py-0.5 text-xs font-medium {styles[
    risk
  ]}"
>
  <span class="size-1.5 rounded-full bg-current" aria-hidden="true"></span>
  {labels[risk]}{#if count !== undefined}&nbsp;·&nbsp;{count}{/if}
</span>
