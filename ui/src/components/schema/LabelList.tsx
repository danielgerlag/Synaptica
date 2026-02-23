import { useState } from 'react'
import { cn } from '@/lib/utils'

interface LabelInfo {
  name: string
  count: number
  properties: string[]
}

interface LabelListProps {
  nodeLabels: LabelInfo[]
  edgeLabels: LabelInfo[]
}

const NODE_COLORS = ['#3b82f6', '#22c55e', '#f59e0b', '#ef4444', '#a855f7', '#06b6d4']
const EDGE_COLORS = ['#f97316', '#ec4899', '#14b8a6', '#8b5cf6', '#eab308', '#64748b']

function LabelCard({
  label,
  color,
  isExpanded,
  onToggle,
}: {
  label: LabelInfo
  color: string
  isExpanded: boolean
  onToggle: () => void
}) {
  return (
    <div
      className={cn(
        'rounded-lg border border-border bg-secondary/30 transition-colors hover:bg-secondary/50',
        isExpanded && 'bg-secondary/50'
      )}
    >
      <button
        onClick={onToggle}
        className="flex w-full items-center gap-3 px-3 py-2.5 text-left"
      >
        <span
          className="h-3 w-3 shrink-0 rounded-full"
          style={{ backgroundColor: color }}
        />
        <span className="flex-1 font-medium text-foreground">{label.name}</span>
        <span className="tabular-nums text-sm text-muted-foreground">
          {label.count.toLocaleString()}
        </span>
        <span className="text-xs text-muted-foreground">{isExpanded ? '▲' : '▼'}</span>
      </button>
      {isExpanded && label.properties.length > 0 && (
        <div className="border-t border-border px-3 py-2">
          <p className="mb-1.5 text-xs font-medium text-muted-foreground">Properties</p>
          <div className="flex flex-wrap gap-1.5">
            {label.properties.map((prop) => (
              <span
                key={prop}
                className="rounded bg-secondary px-2 py-0.5 text-xs text-foreground"
              >
                {prop}
              </span>
            ))}
          </div>
        </div>
      )}
      {isExpanded && label.properties.length === 0 && (
        <div className="border-t border-border px-3 py-2">
          <p className="text-xs italic text-muted-foreground">No properties</p>
        </div>
      )}
    </div>
  )
}

export function LabelList({ nodeLabels, edgeLabels }: LabelListProps) {
  const [expanded, setExpanded] = useState<string | null>(null)

  const toggle = (key: string) => setExpanded((prev) => (prev === key ? null : key))

  return (
    <div className="flex flex-col gap-6">
      <section>
        <h2 className="mb-3 text-sm font-semibold uppercase tracking-wider text-muted-foreground">
          Node Labels
        </h2>
        <div className="flex flex-col gap-2">
          {nodeLabels.map((label, i) => (
            <LabelCard
              key={label.name}
              label={label}
              color={NODE_COLORS[i % NODE_COLORS.length]}
              isExpanded={expanded === `node-${label.name}`}
              onToggle={() => toggle(`node-${label.name}`)}
            />
          ))}
        </div>
      </section>
      <section>
        <h2 className="mb-3 text-sm font-semibold uppercase tracking-wider text-muted-foreground">
          Edge Labels
        </h2>
        <div className="flex flex-col gap-2">
          {edgeLabels.map((label, i) => (
            <LabelCard
              key={label.name}
              label={label}
              color={EDGE_COLORS[i % EDGE_COLORS.length]}
              isExpanded={expanded === `edge-${label.name}`}
              onToggle={() => toggle(`edge-${label.name}`)}
            />
          ))}
        </div>
      </section>
    </div>
  )
}
