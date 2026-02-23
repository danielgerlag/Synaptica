import { Routes, Route, Navigate } from 'react-router-dom'
import { AppLayout } from './components/layout/AppLayout'
import { QueryPage } from './pages/QueryPage'
import { GraphPage } from './pages/GraphPage'
import { SchemaPage } from './pages/SchemaPage'
import { ExplorerPage } from './pages/ExplorerPage'
import { ClusterPage } from './pages/ClusterPage'
import { MetricsPage } from './pages/MetricsPage'

export default function App() {
  return (
    <Routes>
      <Route element={<AppLayout />}>
        <Route path="/" element={<Navigate to="/query" replace />} />
        <Route path="/query" element={<QueryPage />} />
        <Route path="/graph" element={<GraphPage />} />
        <Route path="/schema" element={<SchemaPage />} />
        <Route path="/explorer" element={<ExplorerPage />} />
        <Route path="/cluster" element={<ClusterPage />} />
        <Route path="/metrics" element={<MetricsPage />} />
      </Route>
    </Routes>
  )
}
