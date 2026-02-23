import { NodeCard, type ClusterNodeInfo } from './NodeCard'

interface ClusterOverviewProps {
  nodes: ClusterNodeInfo[]
}

export function ClusterOverview({ nodes }: ClusterOverviewProps) {
  return (
    <div className="flex flex-col gap-3">
      <h2 className="text-lg font-semibold text-foreground">Cluster Nodes</h2>
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {nodes.map((node) => (
          <NodeCard key={node.id} node={node} />
        ))}
      </div>
    </div>
  )
}
