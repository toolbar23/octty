import { describe, expect, test } from "vitest";
import { summarizeUnifiedDiff } from "./diff-stats";

describe("summarizeUnifiedDiff", () => {
  test("counts added and removed content lines while ignoring file headers", () => {
    expect(
      summarizeUnifiedDiff(`diff --git a/a.ts b/a.ts
index 1111111..2222222 100644
--- a/a.ts
+++ b/a.ts
@@ -1,3 +1,4 @@
 line 1
-line 2
+line two
+line 3
 line 4
`),
    ).toEqual({
      addedLines: 2,
      removedLines: 1,
    });
  });

  test("returns zeros for empty input", () => {
    expect(summarizeUnifiedDiff("")).toEqual({
      addedLines: 0,
      removedLines: 0,
    });
  });
});
