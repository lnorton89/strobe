import { describe, expect, it } from "vitest";
import {
  tokenizeShellLine,
  tokenizeUsageLine,
  type ShellToken,
} from "@/lib/shell-syntax";

const compact = (tokens: ShellToken[]) =>
  tokens.filter((t) => t.value.trim().length > 0).map((t) => `${t.kind}:${t.value}`);

describe("tokenizeShellLine", () => {
  it("marks the first word as the command and flags as flags", () => {
    expect(compact(tokenizeShellLine("cargo build --release"))).toEqual([
      "command:cargo",
      "plain:build",
      "flag:--release",
    ]);
  });

  it("starts a new command after an operator", () => {
    expect(compact(tokenizeShellLine("strobe init hello && strobe run hello"))).toEqual([
      "command:strobe",
      "plain:init",
      "plain:hello",
      "operator:&&",
      "command:strobe",
      "plain:run",
      "plain:hello",
    ]);
  });

  it("splits a trailing comment off the command", () => {
    expect(compact(tokenizeShellLine("strobe check hello    # front end only"))).toEqual([
      "command:strobe",
      "plain:check",
      "plain:hello",
      "comment:# front end only",
    ]);
  });

  it("does not treat a mid-word hash as a comment", () => {
    expect(compact(tokenizeShellLine("git show abc#123"))).toEqual([
      "command:git",
      "plain:show",
      "plain:abc#123",
    ]);
  });

  it("does not treat a quoted hash as a comment", () => {
    expect(compact(tokenizeShellLine('echo "a # b"'))).toEqual([
      "command:echo",
      'plain:"a',
      "plain:#",
      'plain:b"',
    ]);
  });

  it("recognises placeholders", () => {
    expect(compact(tokenizeShellLine("strobe fuzz replay <FILE>"))).toEqual([
      "command:strobe",
      "plain:fuzz",
      "plain:replay",
      "placeholder:<FILE>",
    ]);
  });

  it("preserves the line's own spacing", () => {
    const line = "strobe check hello    # front end only";
    expect(tokenizeShellLine(line).map((t) => t.value).join("")).toBe(line);
  });

  it("gives an output line no roles at all", () => {
    expect(tokenizeShellLine("Successfully compiled main.cnb to 'main'.", false)).toEqual([
      { kind: "plain", value: "Successfully compiled main.cnb to 'main'." },
    ]);
  });

  it("returns nothing for an empty line", () => {
    expect(tokenizeShellLine("", false)).toEqual([]);
  });
});

describe("tokenizeUsageLine", () => {
  it("keeps a bracketed group whole even when it contains spaces", () => {
    expect(compact(tokenizeUsageLine("strobe <FILE> [-o|--output PATH]"))).toEqual([
      "command:strobe",
      "placeholder:<FILE>",
      "placeholder:[-o|--output PATH]",
    ]);
  });

  it("marks bare flags outside a group", () => {
    expect(compact(tokenizeUsageLine("strobe build --target host"))).toEqual([
      "command:strobe",
      "plain:build",
      "flag:--target",
      "plain:host",
    ]);
  });

  it("round-trips the line", () => {
    const line = "strobe <COMMAND> [ARGS]";
    expect(tokenizeUsageLine(line).map((t) => t.value).join("")).toBe(line);
  });
});
