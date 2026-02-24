import { useState, useEffect, useCallback } from 'react'
import { client, type ClusterStatus } from '@/lib/grpc-client'
import { ClusterOverview } from '@/components/cluster/ClusterOverview'
import { PartitionMap } from '@/components/cluster/PartitionMap'

const INTERVALS = [5, 10, 30] as const

export function ClusterPage() {
  const [autoRefresh, setAutoRefresh] = useState(false)
  const [interval, setInterval_] = useState<number>(5)
  const [lastRefresh, setLastRefresh] = useState(new Date())
  const [clusterData, setClusterData] = useState<ClusterStatus | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string>()

  const refresh = useCallback(async () => {
    try {
      const status = await client.clusterStatus()
      setClusterData(status)
      setLastRefresh(new Date())
      setError(undefined)
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to fetch cluster status')
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => { refresh() }, [refresh])

  useEffect(() => {
    if (!autoRefresh) return
    const id = globalThis.setInterval(refresh, interval * 1000)
    return () => globalThis.clearInterval(id)
  }, [autoRefresh, interval, refresh])

  const clusterNodes = (clusterData?.nodes ?? []).map((n) => ({
    id: n.id,
    address: n.address,
    role: (n.role === 'Leader' ? 'Leader' : 'Follower') as 'Leader' | 'Follower',
    isHealthy: n.isHealthy,
  }))

  const partitions = Array.from({ length: clusterData?.partitionCount ?? 1 }, (_, i) => ({
    id: `p${i}`,
    rangeStart: `0x${(Math.floor((i / (clusterData?.partitionCount ?? 1)) * 256)).toString(16).padStart(2, '0')}`,
    rangeEnd: `0x${(Math.floor(((i + 1) / (clusterData?.partitionCount ?? 1)) * 256) - 1).toString(16).padStart(2, '0')}`,
    leader: clusterData?.leaderId ?? clusterData?.nodeId ?? 'local',
    replicas: clusterData?.nodes.filter((n) => n.id !== clusterData?.leaderId).map((n) => n.id) ?? [],
  }))

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

      {error && (
        <div className="rounded-lg border border-red-500/30 bg-red-500/10 p-3 text-sm text-red-400">
          {error}
        </div>
      )}

      {loading ? (
        <div className="flex items-center gap-2 py-12 justify-center text-muted-foreground">
          <span className="inline-block h-4 w-4 animate-spin rounded-full border-2 border-current border-t-transparent" />
          Loading cluster status…
        </div>
      ) : (
        <>
          <ClusterOverview nodes={clusterNodes.length > 0 ? clusterNodes : [
            { id: clusterData?.nodeId ?? 'local', address: '0.0.0.0:9090', role: 'Leader' as const, isHealthy: !error },
          ]} />
          <PartitionMap partitions={partitions} />
        </>
      )}
    </div>
  )
}
