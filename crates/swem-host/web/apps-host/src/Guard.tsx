// One panel failing must not take the product with it.
//
// React unmounts the whole tree when a render throws, so a single bad field -
// a list the server left out because it was empty, say - turned the entire
// Workbench into a blank page with no way back. A space that fails now says
// so where it is, and everything else keeps working.

import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  /// What failed, in a person's words: "the Agent space", "the Project space".
  what: string;
  children: ReactNode;
}

interface State {
  problem: string | null;
}

export class Guard extends Component<Props, State> {
  state: State = { problem: null };

  static getDerivedStateFromError(error: unknown): State {
    return { problem: error instanceof Error ? error.message : String(error) };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    // The console is where a developer looks; the page says the short version.
    console.error("SWEM panel failed", error, info.componentStack);
  }

  render(): ReactNode {
    if (this.state.problem === null) return this.props.children;
    return (
      <div className="space panel-failed" data-panel-failed={this.props.what}>
        <p>
          {this.props.what} could not be drawn. The rest of the Workbench still works, and nothing
          recorded was lost - a reload usually brings it back.
        </p>
        <pre className="k-caption">{this.state.problem}</pre>
        <button onClick={() => this.setState({ problem: null })}>Try again</button>
      </div>
    );
  }
}
