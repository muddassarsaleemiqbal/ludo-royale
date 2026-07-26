import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "../../lib/cn";

const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-xl text-sm font-bold transition-[transform,background-color,box-shadow,opacity] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-300 disabled:pointer-events-none disabled:opacity-45 active:scale-[0.98]",
  {
    variants: {
      variant: {
        primary:
          "bg-gradient-to-b from-amber-300 to-amber-500 text-stone-950 shadow-[0_8px_24px_rgba(245,158,11,.24),inset_0_1px_0_rgba(255,255,255,.55)] hover:from-amber-200 hover:to-amber-400",
        secondary:
          "border border-white/10 bg-white/[0.06] text-stone-100 shadow-sm hover:bg-white/10",
        ghost: "text-stone-300 hover:bg-white/[0.06] hover:text-white",
        danger: "bg-red-500/15 text-red-200 hover:bg-red-500/25"
      },
      size: {
        sm: "h-9 px-3",
        md: "h-11 px-5",
        lg: "h-14 px-7 text-base",
        icon: "size-10"
      }
    },
    defaultVariants: {
      variant: "primary",
      size: "md"
    }
  }
);

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean;
}

export function Button({
  className,
  variant,
  size,
  asChild = false,
  ...props
}: ButtonProps) {
  const Component = asChild ? Slot : "button";
  return (
    <Component
      className={cn(buttonVariants({ variant, size }), className)}
      {...props}
    />
  );
}
