import { test } from "node:test";
import assert from "node:assert/strict";
import { memoryDerivedAssistantIds } from "../messages.ts";

test("memory provenance survives later turns and resume, without marking original user/tool evidence", () => {
  const message = (id: string, role: string, toolName?: string) => ({id, type: "message", message: {role, toolName}});
  const branch = [message("original", "assistant"), message("user", "user"),
    {id: "recall", type: "custom_message", customType: "sdp-memory-context"},
    message("answer", "assistant"), message("correction", "user"),
    message("observed", "toolResult", "bash"), message("later", "assistant")];
  assert.deepEqual([...memoryDerivedAssistantIds(branch)], ["answer", "later"]);
  assert.deepEqual([...memoryDerivedAssistantIds(branch.slice(0, 2))], []);
});

test("explicit memory recall also marks subsequent answers", () => {
  const branch = [
    {id: "tool", type: "message", message: {role: "toolResult", toolName: "memory_recall"}},
    {id: "answer", type: "message", message: {role: "assistant"}},
  ];
  assert.deepEqual([...memoryDerivedAssistantIds(branch)], ["answer"]);
});
