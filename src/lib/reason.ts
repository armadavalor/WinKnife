/**
 * Resolve a recommendation's `reasonKey` into text (K-037).
 *
 * Kept free of runtime `$lib` imports so `node --test` can load it; `i18n.ts`
 * wires in its dictionary lookup.
 */
import type { ReasonParam } from "$lib/types";

type Lookup = (key: string) => string | undefined;

interface Reasoned {
  reason: string;
  reasonKey?: string;
  reasonParams?: Record<string, ReasonParam>;
}

/** Text for one key, or `undefined` when it or any nested key is unknown. */
function resolveKey(
  key: string,
  params: Record<string, ReasonParam> | undefined,
  lookup: Lookup,
): string | undefined {
  const template = lookup(key);
  if (template === undefined) {
    return undefined;
  }
  const values: Record<string, string> = {};
  for (const [name, param] of Object.entries(params ?? {})) {
    if (typeof param === "object") {
      const nested = resolveKey(param.key, param.params, lookup);
      if (nested === undefined) {
        return undefined;
      }
      values[name] = nested;
    } else {
      values[name] = String(param);
    }
  }
  return template.replace(/\{(\w+)\}/g, (match, name: string) => values[name] ?? match);
}

/**
 * The translated reason, or the English `reason` when there is no key or any
 * key in the tree is unknown — never an empty string.
 */
export function resolveReason(rec: Reasoned, lookup: Lookup): string {
  if (!rec.reasonKey) {
    return rec.reason;
  }
  return resolveKey(rec.reasonKey, rec.reasonParams, lookup) || rec.reason;
}
