// Small hand-rolled UI primitives.
//
// Deliberately not a component library: the whole app needs a button, an input,
// a checkbox, a select, tabs and a modal, and hand-writing them keeps the boot
// bundle tiny, which is an explicit product goal.

import {
  createContext,
  forwardRef,
  useContext,
  useEffect,
  useId,
  useRef,
  type ButtonHTMLAttributes,
  type InputHTMLAttributes,
  type ReactNode,
  type SelectHTMLAttributes,
} from "react";

import { cn } from "@/lib/utils";

// ---------------------------------------------------------------------------
// Button
// ---------------------------------------------------------------------------

type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";
type ButtonSize = "sm" | "md";

const BUTTON_VARIANTS: Record<ButtonVariant, string> = {
  primary:
    "bg-accent text-accent-fg hover:brightness-110 active:brightness-95 shadow-sm shadow-accent/20",
  secondary: "bg-raised text-fg hover:bg-border border border-border",
  ghost: "text-muted hover:text-fg hover:bg-raised",
  danger: "bg-bad text-white hover:brightness-110",
};

const BUTTON_SIZES: Record<ButtonSize, string> = {
  sm: "h-7 px-2.5 text-xs gap-1.5 rounded-md",
  md: "h-9 px-3.5 text-sm gap-2 rounded-lg",
};

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { className, variant = "secondary", size = "md", ...props },
  ref,
) {
  return (
    <button
      ref={ref}
      className={cn(
        "inline-flex select-none items-center justify-center font-medium transition-[filter,background-color,color] duration-100",
        "disabled:pointer-events-none disabled:opacity-50",
        BUTTON_VARIANTS[variant],
        BUTTON_SIZES[size],
        className,
      )}
      {...props}
    />
  );
});

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

export const Input = forwardRef<HTMLInputElement, InputHTMLAttributes<HTMLInputElement>>(
  function Input({ className, ...props }, ref) {
    return (
      <input
        ref={ref}
        className={cn(
          "h-9 w-full rounded-lg border border-border bg-surface px-3 text-sm text-fg",
          "placeholder:text-muted/70",
          "transition-colors focus:border-accent/60",
          "disabled:opacity-60",
          className,
        )}
        {...props}
      />
    );
  },
);

// ---------------------------------------------------------------------------
// Select
// ---------------------------------------------------------------------------

export interface SelectProps extends SelectHTMLAttributes<HTMLSelectElement> {
  options: Array<{ value: string; label: string }>;
}

export function Select({ className, options, ...props }: SelectProps) {
  return (
    <select
      className={cn(
        "h-8 rounded-md border border-border bg-surface px-2 text-xs text-fg",
        "transition-colors focus:border-accent/60",
        className,
      )}
      {...props}
    >
      {options.map((option) => (
        <option key={option.value} value={option.value}>
          {option.label}
        </option>
      ))}
    </select>
  );
}

// ---------------------------------------------------------------------------
// Checkbox
// ---------------------------------------------------------------------------

export interface CheckboxProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label?: ReactNode;
  description?: ReactNode;
  className?: string;
  disabled?: boolean;
}

export function Checkbox({
  checked,
  onChange,
  label,
  description,
  className,
  disabled,
}: CheckboxProps) {
  const id = useId();

  return (
    <div className={cn("flex items-start gap-2.5", className)}>
      <input
        id={id}
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
        className={cn(
          "mt-0.5 h-4 w-4 shrink-0 cursor-pointer rounded border-border bg-surface",
          "accent-[hsl(var(--accent))]",
          "disabled:cursor-not-allowed disabled:opacity-50",
        )}
      />
      {(label || description) && (
        <div className="min-w-0 leading-tight">
          {label && (
            <label htmlFor={id} className="cursor-pointer text-sm text-fg">
              {label}
            </label>
          )}
          {description && (
            <p className="mt-1 text-xs leading-relaxed text-muted">{description}</p>
          )}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Switch
// ---------------------------------------------------------------------------

export interface SwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  /** Describes what the switch controls, for screen readers. */
  label: string;
  className?: string;
  disabled?: boolean;
}

/** A checkbox reads as one item among many; a switch reads as a feature on or off. */
export function Switch({ checked, onChange, label, className, disabled }: SwitchProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cn(
        "relative h-4 w-7 shrink-0 rounded-full transition-colors",
        "disabled:cursor-not-allowed disabled:opacity-50",
        checked ? "bg-accent" : "bg-border",
        className,
      )}
    >
      <span
        className={cn(
          // Anchored left: without it, the button's centred text alignment decides
          // the knob's static position and the offsets below start from the middle.
          "absolute left-0 top-0.5 h-3 w-3 rounded-full bg-surface shadow-sm transition-transform",
          checked ? "translate-x-3.5" : "translate-x-0.5",
        )}
      />
    </button>
  );
}

// ---------------------------------------------------------------------------
// Modal
// ---------------------------------------------------------------------------

export interface ModalProps {
  open: boolean;
  onClose?: () => void;
  children: ReactNode;
  className?: string;
  /** Blocks Escape and backdrop dismissal, for the boot dialog. */
  mandatory?: boolean;
  labelledBy?: string;
}

export function Modal({
  open,
  onClose,
  children,
  className,
  mandatory,
  labelledBy,
}: ModalProps) {
  const panelRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open || mandatory || !onClose) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, mandatory, onClose]);

  // Move focus into the dialog so keyboard users land in the right place.
  useEffect(() => {
    if (!open) return;
    const target = panelRef.current?.querySelector<HTMLElement>(
      "[data-autofocus], input:not([type=hidden]), button",
    );
    target?.focus();
  }, [open]);

  if (!open) return null;

  return (
    // The backdrop scrolls rather than clipping. `body` has overflow hidden, so a
    // dialog taller than the viewport would otherwise be unreachable — which is
    // reachable in practice: a 1366x768 laptop at 150% scaling leaves only 512 CSS
    // pixels of height, and the boot dialog is close to that.
    <div
      className="fixed inset-0 z-50 overflow-y-auto bg-black/50 animate-fade-in"
      onMouseDown={(event) => {
        if (!mandatory && onClose && event.target === event.currentTarget) onClose();
      }}
    >
      <div
        className="flex min-h-full items-center justify-center p-6"
        onMouseDown={(event) => {
          if (!mandatory && onClose && event.target === event.currentTarget) onClose();
        }}
      >
        <div
          ref={panelRef}
          role="dialog"
          aria-modal="true"
          aria-labelledby={labelledBy}
          className={cn(
            "w-full max-w-md rounded-2xl border border-border bg-surface p-6 shadow-2xl animate-scale-in",
            className,
          )}
        >
          {children}
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Tabs
// ---------------------------------------------------------------------------

interface TabsContextValue {
  value: string;
  setValue: (value: string) => void;
}

const TabsContext = createContext<TabsContextValue | null>(null);

export function Tabs({
  value,
  onValueChange,
  children,
  className,
}: {
  value: string;
  onValueChange: (value: string) => void;
  children: ReactNode;
  className?: string;
}) {
  return (
    <TabsContext.Provider value={{ value, setValue: onValueChange }}>
      <div className={cn("flex min-h-0 flex-col", className)}>{children}</div>
    </TabsContext.Provider>
  );
}

export function TabsList({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div
      role="tablist"
      className={cn("inline-flex gap-1 rounded-lg bg-raised p-1", className)}
    >
      {children}
    </div>
  );
}

export function TabsTrigger({
  value,
  children,
}: {
  value: string;
  children: ReactNode;
}) {
  const context = useContext(TabsContext);
  if (!context) throw new Error("TabsTrigger must be used inside Tabs");
  const active = context.value === value;

  return (
    <button
      role="tab"
      aria-selected={active}
      onClick={() => context.setValue(value)}
      className={cn(
        "rounded-md px-3 py-1.5 text-xs font-medium transition-colors",
        active ? "bg-surface text-fg shadow-sm" : "text-muted hover:text-fg",
      )}
    >
      {children}
    </button>
  );
}

export function TabsContent({
  value,
  children,
  className,
}: {
  value: string;
  children: ReactNode;
  className?: string;
}) {
  const context = useContext(TabsContext);
  if (!context) throw new Error("TabsContent must be used inside Tabs");
  if (context.value !== value) return null;
  return <div className={cn("min-h-0 flex-1", className)}>{children}</div>;
}

// ---------------------------------------------------------------------------
// Misc
// ---------------------------------------------------------------------------

export function Spinner({ className }: { className?: string }) {
  return (
    <svg
      className={cn("h-4 w-4 animate-spin", className)}
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <circle cx="12" cy="12" r="10" stroke="currentColor" strokeOpacity="0.25" strokeWidth="3" />
      <path
        d="M12 2a10 10 0 0 1 10 10"
        stroke="currentColor"
        strokeWidth="3"
        strokeLinecap="round"
      />
    </svg>
  );
}

/** Determinate when a total is known, indeterminate while resolving. */
export function ProgressBar({
  done,
  total,
  className,
}: {
  done: number;
  total: number;
  className?: string;
}) {
  const indeterminate = total <= 0;
  const percent = indeterminate ? 30 : Math.min(100, Math.round((done / total) * 100));

  return (
    <div className={cn("h-1 w-full overflow-hidden rounded-full bg-raised", className)}>
      <div
        className={cn(
          "h-full rounded-full bg-accent transition-[width] duration-200",
          indeterminate && "animate-pulse",
        )}
        style={{ width: `${percent}%` }}
      />
    </div>
  );
}

export function Badge({
  children,
  tone = "neutral",
  className,
}: {
  children: ReactNode;
  tone?: "neutral" | "good" | "bad" | "warn" | "accent";
  className?: string;
}) {
  const tones = {
    neutral: "bg-raised text-muted",
    good: "bg-good/15 text-good",
    bad: "bg-bad/15 text-bad",
    warn: "bg-warn/15 text-warn",
    accent: "bg-accent/15 text-accent",
  } as const;

  return (
    <span
      className={cn(
        "inline-flex items-center rounded-full px-2 py-0.5 text-[11px] font-medium",
        tones[tone],
        className,
      )}
    >
      {children}
    </span>
  );
}

export function EmptyState({
  title,
  hint,
  icon,
}: {
  title: string;
  hint?: ReactNode;
  icon?: ReactNode;
}) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-3 p-8 text-center">
      {icon && <div className="text-muted/50">{icon}</div>}
      <p className="text-sm font-medium text-fg">{title}</p>
      {hint && <p className="max-w-sm text-xs leading-relaxed text-muted">{hint}</p>}
    </div>
  );
}
