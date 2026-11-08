#!/usr/bin/env node
import { realpathSync } from "node:fs";
import { fileURLToPath } from "node:url";

const SEPARATORS = new Set([";", "&&", "||", "|", "&", "\n", "(", ")"]);

/** Splits a command line into words and operators. Quotes group words and hide operators. A comment ends the line. */
function tokenize(command) {
  const tokens = [];
  let word = "";
  let quote = null;
  const flush = () => {
    if (word) tokens.push(word);
    word = "";
  };
  for (let i = 0; i < command.length; i++) {
    const c = command[i];
    if (quote) {
      if (c === quote) quote = null;
      else if (c === "\\" && quote === '"' && i + 1 < command.length) word += command[++i];
      else word += c;
    } else if (c === "$" && !word && (command[i + 1] === "'" || command[i + 1] === '"')) continue;
    else if (c === "'" || c === '"') quote = c;
    else if (c === "\\" && command[i + 1] === "\n") {
      flush();
      i++;
    } else if (c === "\\" && i + 1 < command.length) word += command[++i];
    else if (c === "#" && !word) while (i + 1 < command.length && command[i + 1] !== "\n") i++;
    else if (c === "\n" || c === ";" || c === "|" || c === "&" || c === "(" || c === ")") {
      flush();
      const two = command.slice(i, i + 2);
      if (two === "&&" || two === "||") {
        tokens.push(two);
        i++;
      } else tokens.push(c);
    } else if (/\s/.test(c)) flush();
    else word += c;
  }
  flush();
  return tokens;
}

/**
 * Scans a command line for the `$(...)` and backtick substitutions that the shell runs, at one nesting level.
 * Single quotes keep everything literal. Double quotes hide parentheses but not substitutions, and a single
 * quote inside double quotes is a plain character. With `nested`, the scan stops at the `)` that closes `$(`.
 */
function scan(command, from, nested) {
  const found = [];
  let depth = 1;
  let single = false;
  let double = false;
  for (let i = from; i < command.length; i++) {
    const c = command[i];
    if (single) single = c !== "'";
    else if (c === "\\") i++;
    else if (c === "'" && !double) single = true;
    else if (c === '"') double = !double;
    else if (c === "$" && command[i + 1] === "(") {
      const inner = scan(command, i + 2, true);
      found.push(command.slice(i + 2, inner.end));
      i = inner.end;
    } else if (c === "`") {
      let body = "";
      for (i++; i < command.length && command[i] !== "`"; i++) {
        if (command[i] === "\\" && command[i + 1] === "`") i++;
        body += command[i];
      }
      found.push(body);
    } else if (nested && !double && c === "(") depth++;
    else if (nested && !double && c === ")" && --depth === 0) return { end: i, found };
  }
  return { end: command.length, found };
}

const substitutions = (command) => scan(command, 0, false).found;

/** The first word after the options. `withArg` options take the next word. */
function subcommand(words, withArg) {
  for (let i = 0; i < words.length; i++) {
    if (!words[i].startsWith("-")) return [words[i], words[i + 1]];
    if (withArg.includes(words[i])) i++;
  }
  return [];
}

/**
 * True when the command line runs `git push` or `gh pr create`. It looks at every word, so a wrapper such as
 * `time`, `env -i` or `xargs` does not hide the command. A word inside a quoted string does not count, except
 * the string that follows `-c` (bash -c "...") or `eval`, which is a command line itself. A `$(...)` or backtick
 * substitution is a command line too, also inside double quotes.
 */
export function isGuarded(command) {
  const tokens = tokenize(command);
  if (substitutions(command).some(isGuarded)) return true;
  return tokens.some((t, i) => {
    const name = t.split("/").at(-1);
    const rest = tokens.slice(i + 1);
    if (name === "git" && subcommand(rest, ["-C", "-c", "--git-dir", "--work-tree"])[0] === "push") return true;
    if (name === "gh") {
      const [a, b] = subcommand(rest, ["-R", "--repo"]);
      if (a === "pr" && b === "create") return true;
    }
    const inner = tokens[i + 1];
    return (t === "eval" || /^-[a-z]*c$/.test(t)) && inner !== undefined && isGuarded(inner);
  });
}

export function decision(command) {
  if (!isGuarded(command)) return null;
  return {
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      permissionDecision: "ask",
      permissionDecisionReason: "AgentSI: this command publishes code (push or pull request). Confirm only if the owner asked for it explicitly.",
    },
  };
}

/** Node resolves `import.meta.url` to the real path, so a symlinked plugin directory needs the same for `argv[1]`. */
if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  let input = "";
  process.stdin.setEncoding("utf8");
  for await (const chunk of process.stdin) input += chunk;
  let command = "";
  try {
    command = JSON.parse(input)?.tool_input?.command ?? "";
  } catch {
    // A malformed payload is not a push: let the normal permission flow decide.
  }
  const out = typeof command === "string" ? decision(command) : null;
  if (out) process.stdout.write(JSON.stringify(out));
}
