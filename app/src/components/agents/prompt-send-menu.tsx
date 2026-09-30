"use client";

import {
  type FocusEvent,
  type KeyboardEvent,
  type PointerEvent,
  type ReactNode,
  useEffect,
  useRef,
  useState,
} from "react";
import { cn } from "@/lib/utils";

export interface SendMenuRow {
  id: string;
  label: string;
  /** Shortcut shown as key caps, e.g. ["Ctrl", "↵"]. */
  keys: string[];
  disabled?: boolean;
  onSelect: () => void;
}

const OPEN_DELAY = 250;
const CLOSE_DELAY = 200;
const LONG_PRESS = 450;

/**
 * Wraps the send button. Hovering it (after a short intent delay), focusing it with the keyboard,
 * or pressing and holding it opens a small menu of other ways to send.
 */
export function SendMenu({
  rows,
  className,
  children,
}: {
  rows: SendMenuRow[];
  className?: string;
  children: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const openTimer = useRef<number | undefined>(undefined);
  const closeTimer = useRef<number | undefined>(undefined);
  const pressTimer = useRef<number | undefined>(undefined);
  const suppressClick = useRef(false);
  const focusFirst = useRef(false);
  const available = rows.some((row) => !row.disabled);

  const cancel = () => {
    window.clearTimeout(openTimer.current);
    window.clearTimeout(closeTimer.current);
  };
  const show = (delay: number) => {
    cancel();
    if (delay === 0) setOpen(true);
    else openTimer.current = window.setTimeout(() => setOpen(true), delay);
  };
  const hide = (delay: number) => {
    cancel();
    if (delay === 0) setOpen(false);
    else closeTimer.current = window.setTimeout(() => setOpen(false), delay);
  };
  const inMenu = (target: EventTarget | null) =>
    Boolean(menu.current && target instanceof Node && menu.current.contains(target));
  const items = () =>
    Array.from(menu.current?.querySelectorAll<HTMLButtonElement>('button[role="menuitem"]:not(:disabled)') ?? []);

  useEffect(() => () => {
    window.clearTimeout(openTimer.current);
    window.clearTimeout(closeTimer.current);
    window.clearTimeout(pressTimer.current);
  }, []);

  useEffect(() => {
    if (!available && open) setOpen(false);
  }, [available, open]);

  useEffect(() => {
    if (open && focusFirst.current) items()[0]?.focus();
    focusFirst.current = false;
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const away = (event: globalThis.PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    window.addEventListener("pointerdown", away);
    return () => window.removeEventListener("pointerdown", away);
  }, [open]);

  const trigger = () => root.current?.querySelector<HTMLButtonElement>("button:not([role='menuitem'])");

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape" && open) {
      event.stopPropagation();
      setOpen(false);
      if (inMenu(event.target)) trigger()?.focus();
      return;
    }
    if (!available) return;
    if (event.key === "ArrowUp" && !inMenu(event.target)) {
      event.preventDefault();
      if (open) items()[0]?.focus();
      else {
        focusFirst.current = true;
        show(0);
      }
      return;
    }
    if ((event.key === "ArrowDown" || event.key === "ArrowUp") && inMenu(event.target)) {
      event.preventDefault();
      const list = items();
      const at = list.indexOf(event.target as HTMLButtonElement);
      const next = (at + (event.key === "ArrowDown" ? 1 : -1) + list.length) % list.length;
      list[next]?.focus();
    }
  };

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (!available || inMenu(event.target)) return;
    window.clearTimeout(pressTimer.current);
    pressTimer.current = window.setTimeout(() => {
      suppressClick.current = true;
      show(0);
    }, LONG_PRESS);
  };
  const stopPress = () => window.clearTimeout(pressTimer.current);

  const onBlur = (event: FocusEvent<HTMLDivElement>) => {
    if (!root.current?.contains(event.relatedTarget as Node | null)) hide(120);
  };

  return (
    <div
      ref={root}
      className={cn("send-menu-wrap", className)}
      onPointerEnter={(event) => {
        if (event.pointerType === "mouse" && available) show(OPEN_DELAY);
      }}
      onPointerLeave={(event) => {
        stopPress();
        if (event.pointerType === "mouse") hide(CLOSE_DELAY);
      }}
      onPointerDown={onPointerDown}
      onPointerUp={stopPress}
      onPointerCancel={stopPress}
      onFocus={(event) => {
        if (!available || inMenu(event.target)) return;
        if (event.target instanceof HTMLElement && event.target.matches(":focus-visible")) show(0);
      }}
      onBlur={onBlur}
      onKeyDown={onKeyDown}
      onClickCapture={(event) => {
        if (suppressClick.current) {
          suppressClick.current = false;
          event.preventDefault();
          event.stopPropagation();
        }
      }}
      onClick={(event) => {
        if (!inMenu(event.target)) {
          cancel();
          setOpen(false);
        }
      }}
    >
      {children}
      {open && available ? (
        <div className="send-menu" ref={menu}>
          <div className="send-menu-panel" role="menu" aria-label="Send options">
            {rows.map((row) => (
              <button
                key={row.id}
                type="button"
                role="menuitem"
                disabled={row.disabled}
                onClick={() => {
                  setOpen(false);
                  row.onSelect();
                }}
              >
                <span>{row.label}</span>
                <span className="send-menu-keys" aria-hidden="true">
                  {row.keys.map((key) => (
                    <kbd key={key}>{key}</kbd>
                  ))}
                </span>
              </button>
            ))}
          </div>
        </div>
      ) : null}
    </div>
  );
}
