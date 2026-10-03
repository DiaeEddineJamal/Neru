"use client";
// beui.dev/components/agents/file-diff

import {
  Check,
  ChevronDown,
  Copy,
  FileCode2,
  LoaderCircle,
} from "lucide-react";
import { motion, useReducedMotion } from "motion/react";
import {
  type ReactNode,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  type AgentCodeLanguage,
  AgentCodeLine,
  useAgentCodeTokens,
} from "@/components/agents/agent-code";
import { AgentDisclosure } from "@/components/agents/agent-disclosure";
import { SPRING_PRESS, SPRING_SWAP } from "@/lib/ease";
import { cn } from "@/lib/utils";

export type FileDiffStatus = "streaming" | "complete";
export type FileDiffLineType = "added" | "removed" | "context";

export interface FileDiffLine {
  id: string;
  type?: FileDiffLineType;
  oldLine?: number;
  newLine?: number;
  content: string;
}

export interface FileDiffProps {
  file: ReactNode;
  lines: FileDiffLine[];
  status?: FileDiffStatus;
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  collapseOnComplete?: boolean;
  maxHeight?: number;
  language?: AgentCodeLanguage;
  copyText?: string;
  onCopy?: () => void | Promise<void>;
  className?: string;
  /** Called with the new-file line number when a row is clicked. */
  onLineClick?: (line: number) => void;
  comments?: { line: number; text: string }[];
  /** Side-by-side shows old left, new right; falls back to unified in narrow containers. */
  view?: "unified" | "split";
}

/** Below this container width split view falls back to unified. */
const SPLIT_MIN_WIDTH = 560;

type SplitRow =
  | { hunk: number }
  | { left?: number; right?: number };

/** Pairs each run of removals with the additions that follow it; context sits on both sides. */
export function splitRows(lines: FileDiffLine[]): SplitRow[] {
  const rows: SplitRow[] = [];
  let removed: number[] = [];
  let added: number[] = [];
  const flush = () => {
    for (let i = 0; i < Math.max(removed.length, added.length); i++) {
      rows.push({ left: removed[i], right: added[i] });
    }
    removed = [];
    added = [];
  };
  lines.forEach((line, index) => {
    if (line.type === "removed") {
      if (added.length) flush();
      removed.push(index);
    } else if (line.type === "added") added.push(index);
    else {
      flush();
      rows.push(line.type ? { left: index, right: index } : { hunk: index });
    }
  });
  flush();
  return rows;
}

function ChangeCount({ value, type }: { value: number; type: "added" | "removed" }) {
  if (!value) return null;
  return (
    <span
      className={cn(
        "font-mono text-xs tabular-nums",
        type === "added"
          ? "text-emerald-600 dark:text-emerald-400"
          : "text-rose-600 dark:text-rose-400",
      )}
    >
      {type === "added" ? "+" : "−"}
      {value}
    </span>
  );
}

export function FileDiff({
  file,
  lines,
  status = "streaming",
  open,
  defaultOpen = true,
  onOpenChange,
  collapseOnComplete = true,
  maxHeight = 220,
  language = "typescript",
  copyText,
  onCopy,
  className,
  onLineClick,
  comments = [],
  view = "unified",
}: FileDiffProps) {
  const reduce = useReducedMotion() ?? false;
  const rootRef = useRef<HTMLDivElement>(null);
  const [narrow, setNarrow] = useState(false);
  const split = view === "split" && !narrow;
  const baseId = useId();
  const triggerId = `${baseId}-trigger`;
  const contentId = `${baseId}-content`;
  const viewportRef = useRef<HTMLDivElement>(null);
  const previousStatus = useRef(status);
  const copyTimer = useRef<number | undefined>(undefined);
  const [copied, setCopied] = useState(false);
  const [internalOpen, setInternalOpen] = useState(defaultOpen);
  const currentOpen = open ?? internalOpen;
  const streaming = status === "streaming";
  const additions = lines.filter((line) => line.type === "added").length;
  const deletions = lines.filter((line) => line.type === "removed").length;
  const canCopy = Boolean(copyText || onCopy);
  const code = lines.map((line) => line.content).join("\n");
  const tokens = useAgentCodeTokens(code, language);
  const rows = useMemo(() => (split ? splitRows(lines) : []), [lines, split]);

  const setOpen = useCallback(
    (next: boolean) => {
      if (open === undefined) setInternalOpen(next);
      onOpenChange?.(next);
    },
    [onOpenChange, open],
  );

  useEffect(() => {
    if (previousStatus.current !== "streaming" && status === "streaming") {
      setOpen(true);
    }
    if (
      previousStatus.current === "streaming" &&
      status === "complete" &&
      collapseOnComplete
    ) {
      setOpen(false);
    }
    previousStatus.current = status;
  }, [collapseOnComplete, setOpen, status]);

  useEffect(
    () => () => {
      if (copyTimer.current) window.clearTimeout(copyTimer.current);
    },
    [],
  );

  useEffect(() => {
    const root = rootRef.current;
    if (!root || view !== "split" || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(([entry]) =>
      setNarrow(entry.contentRect.width < SPLIT_MIN_WIDTH),
    );
    observer.observe(root);
    return () => observer.disconnect();
  }, [view]);

  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport || !currentOpen || !streaming) return;

    const frame = requestAnimationFrame(() => {
      if (viewport.scrollHeight <= viewport.clientHeight) return;
      if (typeof viewport.scrollTo === "function") {
        viewport.scrollTo({
          top: viewport.scrollHeight,
          behavior: reduce ? "auto" : "smooth",
        });
      } else {
        viewport.scrollTop = viewport.scrollHeight;
      }
    });
    return () => cancelAnimationFrame(frame);
  });

  const handleCopy = useCallback(async () => {
    if (onCopy) await onCopy();
    else if (copyText) await navigator.clipboard?.writeText(copyText);

    setCopied(true);
    if (copyTimer.current) window.clearTimeout(copyTimer.current);
    copyTimer.current = window.setTimeout(() => setCopied(false), 1600);
  }, [copyText, onCopy]);

  return (
    <div
      ref={rootRef}
      data-state={status}
      data-view={split ? "split" : "unified"}
      aria-busy={streaming}
      className={cn("w-full text-sm", className)}
    >
      <button
        id={triggerId}
        type="button"
        aria-expanded={currentOpen}
        aria-controls={contentId}
        onClick={() => setOpen(!currentOpen)}
        className="group flex min-h-9 w-full items-center gap-2 rounded-md py-1 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
      >
        <FileCode2
          aria-hidden="true"
          className="size-4 shrink-0 text-muted-foreground"
        />
        <span className="min-w-0 flex-1 truncate font-mono text-xs text-foreground/80">
          {file}
        </span>
        <span className="flex shrink-0 items-center gap-2">
          <ChangeCount value={additions} type="added" />
          <ChangeCount value={deletions} type="removed" />
        </span>
        <span className="grid size-4 shrink-0 place-items-center text-muted-foreground/60">
          {streaming ? (
            <LoaderCircle
              aria-label="Applying changes"
              className={cn("size-3.5", !reduce && "animate-spin")}
            />
          ) : (
            <Check aria-label="Changes applied" className="size-3.5" />
          )}
        </span>
        <motion.span
          aria-hidden="true"
          animate={{ rotate: currentOpen ? 180 : 0 }}
          transition={reduce ? { duration: 0 } : SPRING_SWAP}
          className="shrink-0 text-muted-foreground/45 transition-colors group-hover:text-muted-foreground"
        >
          <ChevronDown className="size-3.5" />
        </motion.span>
      </button>

      <AgentDisclosure
        id={contentId}
        role="region"
        aria-labelledby={triggerId}
        open={currentOpen}
      >
        <div className="pl-6 pt-1.5">
          <div className="overflow-hidden rounded-xl bg-muted/80">
            <div
              ref={viewportRef}
              data-slot="file-diff-viewport"
              aria-live="polite"
              className="scrollbar-hide overflow-auto"
              style={{ maxHeight }}
            >
              {split ? (
                <div className="grid grid-cols-2 font-mono text-xs leading-5">
                  <span className="sr-only">File changes, side by side</span>
                  {(["left", "right"] as const).map((side) => (
                    <div
                      key={side}
                      aria-label={side === "left" ? "Before" : "After"}
                      className={cn(
                        "min-w-0 overflow-x-auto",
                        side === "right" && "border-l border-foreground/[0.06]",
                      )}
                    >
                      <div className="w-max min-w-full">
                        {rows.map((row, rowIndex) => {
                          if ("hunk" in row) {
                            return (
                              <div
                                key={rowIndex}
                                className="h-5 whitespace-pre bg-foreground/[0.03] px-2 text-muted-foreground/60"
                              >
                                {side === "left" ? lines[row.hunk].content : " "}
                              </div>
                            );
                          }
                          // Notes belong to the row (same key the click uses) so both sides stay aligned.
                          const anchor = lines[row.right ?? row.left ?? 0];
                          const rowLine = anchor?.newLine ?? anchor?.oldLine;
                          const notes = comments.filter((c) => c.line === rowLine);
                          const index = row[side];
                          const line = index === undefined ? undefined : lines[index];
                          // ponytail: notes stay one line tall in split so both sides keep row alignment
                          const noteRows = notes.map(note => side === "right"
                            ? <div key={note.text} title={note.text} className="h-5 max-w-[40ch] truncate border-l-2 border-emerald-500/50 bg-emerald-500/[0.06] px-3 text-[11px] text-foreground/80">{note.text}</div>
                            : <div key={note.text} className="h-5 bg-emerald-500/[0.03]" />);
                          if (!line) {
                            return (
                              <div key={rowIndex}>
                                <div className="h-5 bg-foreground/[0.03]" />
                                {noteRows}
                              </div>
                            );
                          }
                          const type = line.type ?? "context";
                          const lineNumber =
                            side === "left" ? line.oldLine : line.newLine;
                          const clickLine = line.newLine ?? line.oldLine;
                          return (
                            <div key={rowIndex}>
                              <div
                                role={onLineClick && clickLine ? "button" : undefined}
                                tabIndex={onLineClick && clickLine ? 0 : undefined}
                                onClick={() => { if (clickLine) onLineClick?.(clickLine) }}
                                onKeyDown={event => { if ((event.key === "Enter" || event.key === " ") && clickLine) { event.preventDefault(); onLineClick?.(clickLine) } }}
                                className={cn(
                                  "grid h-5 grid-cols-[2.25rem_1rem_minmax(0,1fr)]",
                                  type === "added" && "bg-emerald-500/[0.07]",
                                  type === "removed" && "bg-rose-500/[0.07]",
                                  onLineClick && "cursor-pointer hover:bg-foreground/[0.04]",
                                )}
                              >
                                <span className="select-none pr-2 text-right tabular-nums text-muted-foreground/40">
                                  {lineNumber}
                                </span>
                                <span
                                  className={cn(
                                    "select-none text-center text-muted-foreground/45",
                                    type === "added" && "text-emerald-600 dark:text-emerald-400",
                                    type === "removed" && "text-rose-600 dark:text-rose-400",
                                  )}
                                >
                                  {type === "added" ? "+" : type === "removed" ? "−" : ""}
                                </span>
                                <AgentCodeLine
                                  code={line.content}
                                  tokens={tokens?.[index!]}
                                  className="min-w-0 whitespace-pre px-1.5"
                                />
                              </div>
                              {noteRows}
                            </div>
                          );
                        })}
                      </div>
                    </div>
                  ))}
                </div>
              ) : (
              <div className="font-mono text-xs leading-5">
                <span className="sr-only">File changes</span>
                {lines.map((line, index) => {
                  const type = line.type ?? "context";
                  const lineNumber = line.newLine ?? line.oldLine;
                  const notes = comments.filter(comment => comment.line === lineNumber);
                  return (
                    <div key={line.id}>
                    <div
                      role={onLineClick && lineNumber ? "button" : undefined}
                      tabIndex={onLineClick && lineNumber ? 0 : undefined}
                      onClick={() => { if (lineNumber) onLineClick?.(lineNumber) }}
                      onKeyDown={event => { if ((event.key === "Enter" || event.key === " ") && lineNumber) { event.preventDefault(); onLineClick?.(lineNumber) } }}
                      className={cn(
                        "grid grid-cols-[2.25rem_2.25rem_1rem_minmax(0,1fr)]",
                        type === "added" && "bg-emerald-500/[0.07]",
                        type === "removed" && "bg-rose-500/[0.07]",
                        onLineClick && "cursor-pointer hover:bg-foreground/[0.04]",
                      )}
                    >
                      <span className="select-none pr-2 text-right tabular-nums text-muted-foreground/40">
                        {line.oldLine}
                      </span>
                      <span className="select-none pr-2 text-right tabular-nums text-muted-foreground/40">
                        {line.newLine}
                      </span>
                      <span
                        className={cn(
                          "select-none text-center text-muted-foreground/45",
                          type === "added" &&
                            "text-emerald-600 dark:text-emerald-400",
                          type === "removed" &&
                            "text-rose-600 dark:text-rose-400",
                        )}
                      >
                        {type === "added"
                          ? "+"
                          : type === "removed"
                            ? "−"
                            : ""}
                      </span>
                      <AgentCodeLine
                        code={line.content}
                        tokens={tokens?.[index]}
                        className="min-w-0 whitespace-pre px-1.5"
                      />
                    </div>
                    {notes.map(note => <div key={note.text} className="border-l-2 border-emerald-500/50 bg-emerald-500/[0.06] px-3 py-1 text-[11px] text-foreground/80">{note.text}</div>)}
                    </div>
                  );
                })}
              </div>
              )}
            </div>

            {canCopy ? (
              <div className="flex justify-end px-2 pb-1.5 pt-1">
                <motion.button
                  type="button"
                  aria-label={copied ? "Copied" : "Copy diff"}
                  title={copied ? "Copied" : "Copy diff"}
                  onClick={handleCopy}
                  whileTap={reduce ? undefined : { scale: 0.9 }}
                  transition={SPRING_PRESS}
                  className="grid size-7 place-items-center rounded-md text-muted-foreground outline-none transition-colors hover:bg-background/70 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
                >
                  {copied ? (
                    <Check className="size-3.5" />
                  ) : (
                    <Copy className="size-3.5" />
                  )}
                </motion.button>
              </div>
            ) : null}
          </div>
        </div>
      </AgentDisclosure>
    </div>
  );
}
