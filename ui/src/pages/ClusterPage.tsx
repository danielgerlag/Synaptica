import { useState, useEffect, useCallback } from 'react'
import { ClusterOverview } from '@/components/cluster/ClusterOverview'
import { PartitionMap } from '@/components/cluster/PartitionMap'

// Single-node default — will be replaced by ClusterStatus RPC data
const SINGLE_NODE_CLUSTER = {
  nodes: [
    { id: 'local', address: '0.0.0.0:9090', role: 'Leader' as const, isHealthy: true },
  ],
  partitions: [
    { id: 'p1', rangeStart: '0x00', rangeEnd: '0xFF', leader: 'local', replicas: [] as string[] },
  ],
}

const INTERVALS = [5, 10, 30] as const

export function ClusterPage() {
  const [autoRefresh, setAutoRefresh] = useState(false)
  const [interval, setInterval_] = useState<number>(5)
  const [lastRefresh, setLastRefresh] = useState(new Date())

  const refresh = useCallback(() => {
    setLastRefresh(new Date())
  }, [])

  useEffect(() => {
    if (!autoRefresh) return
    const id = globalThis.setInterval(refresh, interval * 1000)
    return () => globalThis.clearInterval(id)
  }, [autoRefresh, interval, refresh])

  return (
    <div className="flex h-full flex-col gap-4">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">Cluster Status</h1>
        <div className="flex items-center gap-3">
          <span className="text-xs text-muted-foreground">
            Last: {lastRefresh.toLocaleTimeString()}
          </span>
          <select
            value={interval}
            onChange={(e) => setInterval_(Number(e.target.value))}
            className="rounded border border-border bg-card px-2 py-1 text-sm text-foreground"
          >
            {INTERVALS.map((s) => (
              <option key={s} value={s}>{s}s</option>
            ))}
          </select>
          <button
            onClick={() => setAutoRefresh((v) => !v)}
            className={`rounded px-3 py-1 text-sm font-medium ${
              autoRefresh
                ? 'bg-primary text-primary-foreground'
                : 'border border-border bg-card text-foreground'
            }`}
          >
            {autoRefresh ? 'Auto-Refresh ON' : 'Auto-Refresh OFF'}
          </button>
        </div>
      </div>

      <ClusterOverview nodes={SINGLE_NODE_CLUSTER.nodes} />
      <PartitionMap partitions={SINGLE_NODE_CLUSTER.partitions} />
    </div>
  )
}
