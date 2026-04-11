import type React from "react";

declare global {
  namespace React {
    namespace JSX {
      interface IntrinsicElements {
        "electrobun-webview": React.DetailedHTMLProps<
          React.HTMLAttributes<HTMLElement>,
          HTMLElement
        > & {
          src?: string | null;
          renderer?: "cef" | "native";
          preload?: string | null;
          html?: string | null;
        };
      }
    }
  }
}

export {};
