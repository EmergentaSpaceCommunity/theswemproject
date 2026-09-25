/// An exact record ref, shown short but carried whole in `data-ref`.
export function RefChip({ value, onClick, active }: { value: string; onClick?: () => void; active?: boolean }) {
  const short = value.startsWith("sha256:") ? value.slice(7, 19) : value;
  return (
    <code className={`ref${active ? " active" : ""}`} data-ref={value} title={value} onClick={onClick}>
      {short}
    </code>
  );
}
