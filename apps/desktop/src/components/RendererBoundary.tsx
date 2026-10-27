import { Component, type ErrorInfo, type ReactNode } from "react";

// A renderer exception must not strand a transparent native window. This
// fallback is independent of the normal view tree and has its own backplate.
export class RendererBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(cause: unknown) {
    return { error: cause instanceof Error ? cause : new Error(String(cause)) };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("UltraVox renderer failed", error, info.componentStack);
  }

  render() {
    const error = this.state.error;
    if (!error) return this.props.children;
    return (
      <main className="renderer-fallback">
        <div role="alert">
          <h1>UltraVox could not render</h1>
          <p>{error.name}: {error.message}</p>
        </div>
        <button type="button" className="btn" onClick={() => window.location.reload()}>
          Reload window
        </button>
      </main>
    );
  }
}
