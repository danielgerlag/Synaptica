const COLORS = [
  '#3b82f6', '#ef4444', '#22c55e', '#f59e0b', '#8b5cf6', '#ec4899',
  '#06b6d4', '#f97316', '#14b8a6', '#6366f1', '#84cc16', '#e11d48',
]

function labelColor(label: string): string {
  let h = 0
  for (let i = 0; i < label.length; i++) h = label.charCodeAt(i) + ((h << 5) - h)
  return COLORS[Math.abs(h) % COLORS.length]
}

interface GraphControlsProps {
  labels: string[]
  visibleLabels: Set<string>
  onToggleLabel: (label: string) => void
  onFitToScreen: () => void
  onResetLayout: () => void
  nodeCount: number
  edgeCount: number
}

export function GraphControls({
  labels,
  visibleLabels,
  onToggleLabel,
  onFitToScreen,
  onResetLayout,
  nodeCount,
  edgeCount,
}: GraphControlsProps) {
  return (
    <div className="flex flex-wrap items-center gap-3 rounded-lg border border-border bg-card px-4 py-2">
      <button
        onClick={onFitToScreen}
        className="rounded-md bg-muted px-3 py-1.5 text-xs font-medium text-foreground hover:bg-muted/80"
      >
        Fit to Screen
      </button>
      <button
        onClick={onResetLayout}
        className="rounded-md bg-muted px-3 py-1.5 text-xs font-medium text-foreground hover:bg-muted/80"
      >
        Reset Layout
      </button>

      <div className="mx-2 h-5 w-px bg-border" />

      {labels.map((label) => (
        <label key={label} className="flex items-center gap-1.5 text-xs cursor-pointer">
          <input
            type="checkbox"
            checked={visibleLabels.has(label)}
            onChange={() => onToggleLabel(label)}
            className="rounded"
          />
          <span
            className="inline-block h-2.5 w-2.5 rounded-full"
            style={{ backgroundColor: labelColor(label) }}
          />
          <span className="text-foreground">{label}</span>
        </label>
      ))}

      <div className="ml-auto text-xs text-muted-foreground">
        {nodeCount} nodes, {edgeCount} edges
      </div>
    </div>
  )
}
