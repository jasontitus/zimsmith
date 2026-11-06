import type { ComponentProps } from "react";

type NumberInputProps = Omit<
  ComponentProps<"input">,
  "type" | "value" | "onChange"
> & {
  value: number | null;
  onValueChange: (value: number) => void;
};

export function NumberInput({
  value,
  onValueChange,
  ...props
}: NumberInputProps) {
  return (
    <input
      {...props}
      type="number"
      value={value !== null && Number.isFinite(value) ? value : ""}
      onChange={(event) => onValueChange(event.currentTarget.valueAsNumber)}
    />
  );
}
