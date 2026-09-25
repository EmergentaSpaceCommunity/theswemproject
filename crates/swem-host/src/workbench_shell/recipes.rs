//! A person runs what a package wrote down.
//!
//! A recipe names no operation of its own: every step is one tool call on the
//! project's own server, through the same door a person's click on a rung's
//! action goes through. So running one is [`WorkbenchShellState::project_tool_call`]
//! in a loop, in the order the package declared, with each step's result kept
//! so a later step may name it. The host decides nothing about what a recipe
//! does - it reads the list from the seam vocabulary, which is the packages'
//! own declaration.
use super::*;

/// One recipe, as a surface offers it. The steps carry what a person is told
//  while each runs, not their arguments: what a step passes is the package's
/// business and unreadable to the person anyway.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecipeView {
    pub name: String,
    pub module: String,
    pub title: String,
    pub summary: String,
    pub starts_a_project: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent_example: Option<String>,
    pub steps: Vec<String>,
}

/// What one step did.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecipeStepRun {
    pub note: String,
    pub tool: String,
    /// `done`, `failed`, or `not run` for a step after the one that failed.
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What a run of a recipe did, step by step.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecipeRun {
    pub recipe: String,
    pub title: String,
    /// True when every step is `done`.
    pub completed: bool,
    pub steps: Vec<RecipeStepRun>,
}

/// The value a later step sees when it names this one: the tool's structured
/// result, which is the record the Cycle wrote, rather than the transport's
/// envelope around it.
fn step_result(result: &Value) -> Value {
    result
        .get("structuredContent")
        .cloned()
        .unwrap_or_else(|| result.clone())
}

impl WorkbenchShellState {
    /// Every recipe a person could run here: the ones this process assembled,
    /// plus the ones of packages installed into it since it started.
    ///
    /// The two sources are not the same set. `assembly::configure` fixes this
    /// process's module set once, so a package installed into a running host
    /// never joins the host's own vocabulary - and yet it does reach the Cycle
    /// of every project opened afterwards, which is where a recipe actually
    /// runs. Reading the installed packages' manifests as well is what lets a
    /// person run what they just installed instead of restarting first.
    fn declared_recipes(&self) -> Vec<crate::SupplyRecipe> {
        self.supply().recipes(self.packages_home())
    }

    /// Every recipe every loaded package brings.
    #[must_use]
    pub fn recipes(&self) -> Vec<RecipeView> {
        self.declared_recipes()
            .into_iter()
            .map(|recipe| RecipeView {
                name: recipe.name,
                module: recipe.module,
                title: recipe.title,
                summary: recipe.summary,
                starts_a_project: recipe.starts_a_project,
                intent_example: recipe.intent_example,
                steps: recipe.steps.into_iter().map(|step| step.note).collect(),
            })
            .collect()
    }

    /// Run one recipe on one project, stopping at the first step that fails.
    ///
    /// A failed step is reported with the tool's own refusal and the steps
    /// after it are reported as not run, because a recipe is an order: what
    /// follows a step that did not happen would be acting on a project that
    /// is not the one the package described.
    ///
    /// # Errors
    ///
    /// Refuses a recipe no loaded package declares (`NotFound`). A step that
    /// fails is not an error of the run: it comes back inside the answer, so
    /// the person sees which step and why.
    pub async fn run_recipe(
        &self,
        server: &str,
        name: &str,
    ) -> Result<RecipeRun, WorkbenchShellError> {
        let recipe = self
            .declared_recipes()
            .into_iter()
            .find(|recipe| recipe.name == name)
            .ok_or_else(|| {
                WorkbenchShellError::NotFound(format!(
                    "no loaded package declares the recipe {name}"
                ))
            })?;
        let mut results: Vec<Value> = Vec::new();
        let mut steps: Vec<RecipeStepRun> = Vec::new();
        let mut failed = false;
        for (index, step) in recipe.steps.iter().enumerate() {
            if failed {
                steps.push(RecipeStepRun {
                    note: step.note.clone(),
                    tool: step.tool.clone(),
                    outcome: "not run".into(),
                    error: None,
                });
                continue;
            }
            let arguments = match self.supply().recipe_step_arguments(
                self.packages_home(),
                name,
                index + 1,
                &results,
            ) {
                Ok(arguments) => arguments,
                Err(reason) => {
                    failed = true;
                    steps.push(RecipeStepRun {
                        note: step.note.clone(),
                        tool: step.tool.clone(),
                        outcome: "failed".into(),
                        error: Some(reason),
                    });
                    continue;
                }
            };
            match self.project_tool_call(server, &step.tool, arguments).await {
                Ok(result) => {
                    results.push(step_result(&result));
                    steps.push(RecipeStepRun {
                        note: step.note.clone(),
                        tool: step.tool.clone(),
                        outcome: "done".into(),
                        error: None,
                    });
                }
                Err(error) => {
                    failed = true;
                    steps.push(RecipeStepRun {
                        note: step.note.clone(),
                        tool: step.tool.clone(),
                        outcome: "failed".into(),
                        error: Some(error.to_string()),
                    });
                }
            }
        }
        Ok(RecipeRun {
            recipe: recipe.name.clone(),
            title: recipe.title.clone(),
            completed: !failed,
            steps,
        })
    }
}
