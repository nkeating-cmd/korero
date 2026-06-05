import React from "react";

// Kōrero (v1.7.0, B4): React ErrorBoundary — catches render-phase exceptions
// in the React tree and shows a fallback rather than leaving the user with a
// blank window. Class component is required by React's error boundary API;
// hooks-based alternatives cannot implement getDerivedStateFromError.
//
// Usage:
//   <ErrorBoundary>
//     <YourComponent />
//   </ErrorBoundary>
//
// Or with a custom fallback:
//   <ErrorBoundary fallback={<div>Something went wrong.</div>}>
//     <YourComponent />
//   </ErrorBoundary>

interface Props {
  children: React.ReactNode;
  fallback?: React.ReactNode;
}

interface State {
  hasError: boolean;
  error: Error | null;
}

export class ErrorBoundary extends React.Component<Props, State> {
  constructor(props: Props) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo): void {
    // Errors here are unrecoverable React render failures — log them so they
    // appear in the Kōrero log for post-mortem inspection.
    console.error("[Kōrero] React render error caught by ErrorBoundary:", error, info);
  }

  render(): React.ReactNode {
    if (this.state.hasError) {
      if (this.props.fallback) {
        return this.props.fallback;
      }
      return (
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            justifyContent: "center",
            height: "100%",
            padding: "2rem",
            color: "var(--text-muted, #888)",
            fontSize: "0.85rem",
            textAlign: "center",
            gap: "0.5rem",
          }}
        >
          <span style={{ fontSize: "1.5rem" }}>⚠</span>
          <span>Something went wrong. Restart Kōrero to recover.</span>
          {this.state.error && (
            <span
              style={{
                fontFamily: "Consolas, monospace",
                fontSize: "0.75rem",
                opacity: 0.6,
                maxWidth: "320px",
                wordBreak: "break-word",
              }}
            >
              {this.state.error.message}
            </span>
          )}
        </div>
      );
    }

    return this.props.children;
  }
}

export default ErrorBoundary;
