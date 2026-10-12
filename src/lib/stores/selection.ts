/**
 * Which cleaner rows start ticked after a scan.
 *
 * Kept free of Svelte and `$lib` imports so `node --test` can run it directly.
 */
import type { Finding, Recommendation } from "../types.ts";

/**
 * Green, actionable rows start selected; yellow and info-only rows do not
 * (spec 02). The recycle bin never does: emptying it is permanent, so the
 * user ticks it themselves and then confirms the warning.
 */
export function initialSelection(
  recommendations: readonly Recommendation[],
  findings: readonly Finding[],
): Set<string> {
  const kindById = new Map(findings.map((finding) => [finding.id, finding.kind]));
  return new Set(
    recommendations
      .filter((rec) => {
        const kind = kindById.get(rec.findingIds[0]);
        return (
          kind !== undefined &&
          kind !== "recycleBinItem" &&
          rec.risk === "safe" &&
          rec.action !== "infoOnly"
        );
      })
      .map((rec) => rec.id),
  );
}
