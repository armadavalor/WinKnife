import { redirect } from "@sveltejs/kit";

/** The shell has no root screen of its own; the dashboard is the entry point. */
export function load(): never {
  redirect(307, "/dashboard");
}
