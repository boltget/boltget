// The extension table of the categories in Settings, reactive: views and
// icons that read it follow a change of a category's file types at once.
import type { Category as CategorySetting } from "../bindings";
import { DEFAULT_CATEGORIES, tableOf, type Category } from "./category";

let table = $state.raw<ReadonlyMap<string, Category> | null>(null);
/** The built-in table, built on first use (not state: never written in a read). */
let builtin: ReadonlyMap<string, Category> | null = null;

/** The current table (the built-in one until Settings arrive). */
export function categoryTable(): ReadonlyMap<string, Category> {
  return table ?? (builtin ??= tableOf(DEFAULT_CATEGORIES));
}

/** Uses these categories from now on. */
export function setCategories(categories: readonly CategorySetting[]): void {
  table = tableOf(categories);
}
