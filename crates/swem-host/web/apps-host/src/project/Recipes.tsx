// What the installed packages say to do with this project. A recipe is an
// ordered list of the project's own tool calls with the arguments a package
// wrote down: running one does nothing a person could not do from
// the rails by hand - it knows the order. The seeds are offered where a
// project is made, not here: they only make sense on a project whose only
// record is what was asked.
import { useCallback, useEffect, useState } from "react";

import { fetchJson, runRecipe } from "./api";
import type { RecipeRun } from "./api";

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

interface Props {
  serverName: string;
  onChanged: () => void;
}

export function Recipes({ serverName, onChanged }: Props) {
  const [all] = useRecipes();
  const recipes = all.filter((recipe) => !recipe.starts_a_project);
  const [run, setRun] = useState<RecipeRun | null>(null);
  const [running, setRunning] = useState<string | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  if (recipes.length === 0) return null;

  const start = async (recipe: RecipeView) => {
    setRunning(recipe.name);
    setProblem(null);
    setRun(null);
    try {
      setRun(await runRecipe(serverName, recipe.name));
      // Every step wrote, or the ones before the failure did.
      onChanged();
    } catch (failure) {
      setProblem((failure as Error).message);
    } finally {
      setRunning(null);
    }
  };

  return (
    <section id="recipes" aria-label="Recipes">
      <h3>Recipes</h3>
      <p className="k-muted">What the installed packages say to do with a project like this one.</p>
      <div className="recipes-body">
        {recipes.map((recipe) => (
          <div key={recipe.name} className="recipe-row" data-recipe={recipe.name}>
            <div>
              <strong>{recipe.title}</strong>
              <small className="k-muted"> · {recipe.module}</small>
              <p>{recipe.summary}</p>
              <ol className="recipe-steps">
                {recipe.steps.map((step, index) => (
                  <li key={`${recipe.name}-${String(index)}`}>{step}</li>
                ))}
              </ol>
            </div>
            <button
              className="recipe-run"
              data-run={recipe.name}
              disabled={running !== null}
              onClick={() => void start(recipe)}
            >
              {running === recipe.name ? "Running…" : "Run"}
            </button>
          </div>
        ))}
      </div>
      {problem ? <div className="bad">{problem}</div> : null}
      {run ? (
        <div id="recipe-run" className="recipe-run-report" data-completed={run.completed ? "true" : "false"}>
          <strong>{run.completed ? `${run.title}: done` : `${run.title}: stopped`}</strong>
          <ol>
            {run.steps.map((step, index) => (
              <li key={`${step.tool}-${String(index)}`} data-outcome={step.outcome}>
                {step.note} — {step.outcome}
                {step.error ? <div className="bad">{step.error}</div> : null}
              </li>
            ))}
          </ol>
        </div>
      ) : null}
    </section>
  );
}
