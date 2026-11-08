export function SummaryTile({ label, value, total }: { label: string; value: string | number; total?: number }) {
  return (
    <div className="rounded-lg bg-card px-4 py-3.5 shadow-soft">
      <p className="label text-muted-foreground">{label}</p>
      <p className="mt-1 whitespace-nowrap text-2xl font-semibold tabular-nums tracking-[-0.02em]">
        {value}
        {total !== undefined && <span className="text-base font-normal text-muted-foreground">/{total}</span>}
      </p>
    </div>
  );
}
