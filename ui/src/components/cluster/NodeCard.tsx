import { cn } from '@/lib/utils'

export interface ClusterNodeInfo {
  id: string
  address: string
  role: 'Leader' | 'Follower'
  isHealthy: boolean
}

interface NodeCardProps {
  node: ClusterNodeInfo
}

export function NodeCard({ node }: NodeCardProps) {
  const healthColor = node.isHealthy ? 'bg-green-500' : 'bg-red-500'
  const healthLabel = node.isHealthy ? 'Healthy' : 'Unreachable'

  return (
    <div className="rounded-lg border border-border bg-card p-4 flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <span className="font-bold text-foreground">{node.id}</span>
        <span
          className={cn(
            'rounded-full px-2 py-0.5 text-xs font-medium',
            node.role === 'Leader'
              ? 'bg-primary/20 text-primary'
              : 'bg-secondary text-secondary-foreground',
          )}
        >
          {node.role}
        </span>
      </div>
      <span className="text-xs text-muted-foreground font-mono">{node.address}</span>
      <div className="flex items-center gap-2 mt-1">
        <span className={cn('h-2.5 w-2.5 rounded-full', healthColor)} />
        <span className="text-xs text-muted-foreground">{healthLabel}</span>
      </div>
    </div>
  )
}
