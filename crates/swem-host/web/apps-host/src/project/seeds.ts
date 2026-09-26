// What the installed packages say a project of their kind starts as. A seed
// is a recipe with `starts_a_project`: an ordered list of the project's own
// tool calls a package wrote down, run once on a project whose only record is
// what was asked. Seeds are offered where a project is made; the other
// recipes a package ships are data an agent reads from the project's
// vocabulary and performs, not a panel of this page.
import { useCallback, useEffect, useState } from "react";

import { fetchJson } from "./api";

export interface RecipeView {
  name: string;
  module: string;
  title: string;
  summary: string;
  starts_a_project: boolean;
  intent_example?: string;
  /// One line per step, in order: what it does, not what it passes.
  steps: string[];
}

/// Every recipe the loaded packages bring, and the way to read them again:
/// installing a package is how a recipe arrives, so the list a person is
/// offered has to change without them reloading the page.
export function useRecipes(): [RecipeView[], () => void] {
  const [recipes, setRecipes] = useState<RecipeView[]>([]);
  const load = useCallback(() => {
    fetchJson<RecipeView[]>("/api/recipes")
      .then(setRecipes)
      .catch(() => setRecipes([]));
  }, []);
  useEffect(load, [load]);
  return [recipes, load];
}
