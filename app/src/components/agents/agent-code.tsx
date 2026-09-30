"use client";

import {
  type CSSProperties,
  Fragment,
  useEffect,
  useState,
} from "react";
import {
  type BundledLanguage,
  bundledLanguages,
  createHighlighter,
  type Highlighter,
} from "shiki";
import { createJavaScriptRegexEngine } from "shiki/engine/javascript";
import { cn } from "@/lib/utils";

/** A Shiki language id ("tsx", "python", …) or "text" for plain output. */
export type AgentCodeLanguage = string;

// Loaded with the highlighter; anything else in Shiki's bundle loads the first time it is seen.
const PRELOADED_LANGUAGES = ["bash", "css", "html", "javascript", "json", "tsx", "typescript"];

const LANGUAGE_BY_EXTENSION: Record<string, AgentCodeLanguage> = {
  astro: "astro", bat: "bat", c: "c", cc: "cpp", cjs: "javascript", cmd: "bat", cpp: "cpp",
  cs: "csharp", css: "css", cts: "typescript", dart: "dart", diff: "diff", dockerfile: "docker",
  ex: "elixir", exs: "elixir", go: "go", gql: "graphql", graphql: "graphql", h: "c", hpp: "cpp",
  htm: "html", html: "html", ini: "ini", java: "java", js: "javascript", json: "json",
  jsonc: "jsonc", jsx: "jsx", kt: "kotlin", less: "less", lua: "lua", md: "markdown",
  mdx: "mdx", mjs: "javascript", mts: "typescript", php: "php", patch: "diff", ps1: "powershell",
  psm1: "powershell", py: "python", r: "r", rb: "ruby", rs: "rust", sass: "sass", scss: "scss",
  sh: "bash", sql: "sql", svelte: "svelte", svg: "xml", swift: "swift", toml: "toml",
  ts: "typescript", tsx: "tsx", txt: "text", vue: "vue", xml: "xml", yaml: "yaml", yml: "yaml",
  zig: "zig", zsh: "bash",
};

/** Neru addition: pick a highlight language from a file path. */
export function languageForPath(path: string): AgentCodeLanguage {
  const name = path.split(/[\\/]/).pop()?.toLowerCase() ?? "";
  if (name === "dockerfile") return "docker";
  if (name === "makefile") return "make";
  const extension = name.includes(".") ? name.split(".").pop() ?? "" : "";
  return LANGUAGE_BY_EXTENSION[extension] ?? "text";
}

/** Maps a Markdown fence tag ("js", "sh", "py", …) onto a Shiki language id. */
export function languageForFence(tag: string | undefined): AgentCodeLanguage {
  const value = (tag ?? "").trim().toLowerCase();
  if (!value) return "text";
  const alias: Record<string, string> = {
    console: "bash", js: "javascript", node: "javascript", plaintext: "text", ps: "powershell",
    ps1: "powershell", pwsh: "powershell", py: "python", rb: "ruby", rs: "rust", shell: "bash",
    sh: "bash", ts: "typescript", txt: "text", yml: "yaml", zsh: "bash",
  };
  return alias[value] ?? LANGUAGE_BY_EXTENSION[value] ?? value;
}

export interface AgentCodeToken {
  content: string;
  offset: number;
  light?: string;
  dark?: string;
}

export type AgentCodeTokenLines = AgentCodeToken[][];

export interface AgentCodeProps {
  code: string;
  language?: AgentCodeLanguage;
  className?: string;
}

export interface AgentCodeLineProps {
  code: string;
  tokens?: AgentCodeToken[];
  className?: string;
}

// Neru: VS Code's default Light+ and Dark+ colors, so generated code reads like the editor.
const LIGHT_THEME = "light-plus";
const DARK_THEME = "dark-plus";
let agentCodeHighlighter: Promise<Highlighter> | null = null;
const tokenCache = new Map<string, AgentCodeTokenLines>();
const TOKEN_CACHE_LIMIT = 400;
const loadingLanguages = new Map<string, Promise<boolean>>();

function getAgentCodeHighlighter() {
  if (!agentCodeHighlighter) {
    // Neru: the JavaScript regex engine avoids WebAssembly, which the desktop CSP blocks.
    agentCodeHighlighter = createHighlighter({
      themes: [LIGHT_THEME, DARK_THEME],
      langs: PRELOADED_LANGUAGES,
      engine: createJavaScriptRegexEngine({ forgiving: true }),
    });
  }
  return agentCodeHighlighter;
}

/** Loads a language on first use; resolves false when Shiki does not know it. */
function ensureLanguage(highlighter: Highlighter, language: AgentCodeLanguage) {
  if (language === "text" || highlighter.getLoadedLanguages().includes(language)) {
    return Promise.resolve(language !== "text");
  }
  let loading = loadingLanguages.get(language);
  if (!loading) {
    loading = language in bundledLanguages
      ? highlighter.loadLanguage(language as BundledLanguage).then(() => true, () => false)
      : Promise.resolve(false);
    loadingLanguages.set(language, loading);
  }
  return loading;
}

function rememberTokens(key: string, lines: AgentCodeTokenLines) {
  tokenCache.set(key, lines);
  // Streaming code produces a new key per update; keep only the recent ones.
  while (tokenCache.size > TOKEN_CACHE_LIMIT) {
    const oldest = tokenCache.keys().next().value;
    if (oldest === undefined) break;
    tokenCache.delete(oldest);
  }
}

function tokenCacheKey(code: string, language: AgentCodeLanguage) {
  return `${language}\u0000${code}`;
}

export function useAgentCodeTokens(
  code: string,
  language: AgentCodeLanguage,
) {
  const key = tokenCacheKey(code, language);
  const cached = tokenCache.get(key);
  const [result, setResult] = useState<{
    key: string;
    code: string;
    language: AgentCodeLanguage;
    lines: AgentCodeTokenLines;
  } | null>(cached ? { key, code, language, lines: cached } : null);

  useEffect(() => {
    const current = tokenCache.get(key);
    if (current) {
      setResult({ key, code, language, lines: current });
      return;
    }

    let cancelled = false;
    getAgentCodeHighlighter().then(async (highlighter) => {
      const known = await ensureLanguage(highlighter, language);
      if (cancelled) return;
      const lines = highlighter
        .codeToTokensWithThemes(code, {
          lang: (known ? language : "text") as BundledLanguage | "text",
          themes: {
            light: LIGHT_THEME,
            dark: DARK_THEME,
          },
        })
        .map((line) =>
          line.map((token) => ({
            content: token.content,
            offset: token.offset,
            light: token.variants.light?.color,
            dark: token.variants.dark?.color,
          })),
      );
      rememberTokens(key, lines);
      setResult({ key, code, language, lines });
    });
    return () => {
      cancelled = true;
    };
  }, [code, key, language]);

  if (result?.key === key) return result.lines;
  if (result?.language === language && code.startsWith(result.code)) {
    return result.lines;
  }
  return null;
}

export function AgentCodeLine({
  code,
  tokens,
  className,
}: AgentCodeLineProps) {
  return (
    <span className={className}>
      {tokens
        ? tokens.map((token) => (
            <span
              key={`${token.offset}-${token.content}`}
              style={
                {
                  "--agent-code-light": token.light ?? "currentColor",
                  "--agent-code-dark": token.dark ?? token.light ?? "currentColor",
                } as CSSProperties
              }
              className="text-[var(--agent-code-light)] dark:text-[var(--agent-code-dark)]"
            >
              {token.content}
            </span>
          ))
        : code}
    </span>
  );
}

export function AgentCode({
  code,
  language = "bash",
  className,
}: AgentCodeProps) {
  const tokens = useAgentCodeTokens(code, language);
  let offset = 0;
  const lines = code.split("\n").map((content) => {
    const line = { content, offset };
    offset += content.length + 1;
    return line;
  });

  return (
    <pre
      className={cn(
        "m-0 overflow-x-auto whitespace-pre font-mono text-xs leading-5 text-foreground/85",
        className,
      )}
    >
      <code>
        {lines.map((line, index) => (
          <Fragment key={line.offset}>
            <AgentCodeLine code={line.content} tokens={tokens?.[index]} />
            {index < lines.length - 1 ? "\n" : null}
          </Fragment>
        ))}
      </code>
    </pre>
  );
}
