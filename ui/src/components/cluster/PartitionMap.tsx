import { cn } from '@/lib/utils'

export interface PartitionInfo {
  id: string
  rangeStart: string
  rangeEnd: string
  leader: string
  replicas: string[]
}

interface PartitionMapProps {
  partitions: PartitionInfo[]
}

const NODE_COLORS: Record<string, string> = {
  'node-1': 'bg-blue-500',
  'node-2': 'bg-emerald-500',
  'node-3': 'bg-amber-500',
}

function colorForNode(nodeId: string): string {
  return NODE_COLORS[nodeId] ?? 'bg-purple-500'
}

export function PartitionMap({ partitions }: PartitionMapProps) {
  const totalRange = 0xff

  return (
    <div className="flex flex-col gap-3">
      <h2 className="text-lg font-semibold text-foreground">Partition Map</h2>

      {/* Horizontal bar */}
      <div className="flex h-10 w-full overflow-hidden rounded-md border border-border">
        {partitions.map((p) => {
          const start = parseInt(p.rangeStart, 16)
          const end = parseInt(p.rangeEnd, 16)
          const width = ((end - start) / totalRange) * 100

          return (
            <div
              key={p.id}
              className={cn('flex items-center justify-center text-xs font-medium text-white', colorForNode(p.leader))}
              style={{ width: `${width}%` }}
              title={`${p.id}: [${p.rangeStart}, ${p.rangeEnd}) → ${p.leader}`}
            >
              {p.id}
            </div>
          )
        })}
      </div>

      {/* Legend table */}
      <div className="rounded-lg border border-border bg-card overflow-hidden">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-border text-left text-muted-foreground">
              <th className="px-4 py-2 font-medium">Partition</th>
              <th className="px-4 py-2 font-medium">Range</th>
              <th className="px-4 py-2 font-medium">Leader</th>
              <th className="px-4 py-2 font-medium">Replicas</th>
            </tr>
          </thead>
          <tbody>
            {partitions.map((p) => (
              <tr key={p.id} className="border-b border-border last:border-0">
                <td className="px-4 py-2 flex items-center gap-2">
                  <span className={cn('h-3 w-3 rounded-sm', colorForNode(p.leader))} />
                  {p.id}
                </td>
                <td className="px-4 py-2 font-mono text-muted-foreground">
                  [{p.rangeStart}, {p.rangeEnd})
                </td>
                <td className="px-4 py-2">{p.leader}</td>
                <td className="px-4 py-2 text-muted-foreground">
                  {p.replicas.length > 0 ? p.replicas.join(', ') : '—'}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}
