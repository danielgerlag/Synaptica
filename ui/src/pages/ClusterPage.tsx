import { useState, useEffect, useCallback } from 'react'
import { ClusterOverview } from '@/components/cluster/ClusterOverview'
import { PartitionMap } from '@/components/cluster/PartitionMap'

const DEMO_CLUSTER = {
  nodes: [
    { id: 'node-1', address: '10.0.1.1:9090', role: 'Leader' as const, isHealthy: true },
    { id: 'node-2', address: '10.0.1.2:9090', role: 'Follower' as const, isHealthy: true },
    { id: 'node-3', address: '10.0.1.3:9090', role: 'Follower' as const, isHealthy: false },
  ],
  partitions: [
    { id: 'p1', rangeStart: '0x00', rangeEnd: '0x55', leader: 'node-1', replicas: ['node-2'] },
    { id: 'p2', rangeStart: '0x55', rangeEnd: '0xAA', leader: 'node-2', replicas: ['node-1'] },
    { id: 'p3', rangeStart: '0xAA', rangeEnd: '0xFF', leader: 'node-1', replicas: ['node-3'] },
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

      <ClusterOverview nodes={DEMO_CLUSTER.nodes} />
      <PartitionMap partitions={DEMO_CLUSTER.partitions} />
    </div>
  )
}
