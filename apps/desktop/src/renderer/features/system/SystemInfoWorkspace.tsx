import type { ReactNode } from 'react'
import type { ConnectionProfile, SessionSnapshot, SystemMetrics } from '@termdock/core'
import { t } from '../../i18n'

export function SystemInfoWorkspace({
  activeProfile,
  activeSession
}: {
  activeProfile: ConnectionProfile | null
  activeSession: SessionSnapshot | null
}) {
  const metrics = activeSession?.systemMetrics

  if (!activeSession || !metrics) {
    return (
      <section className="system-info-workspace system-info-workspace-empty">
        <div className="system-info-empty">
          <strong>{t.noConnection}</strong>
          <p>{t.noConnectionDescription}</p>
        </div>
      </section>
    )
  }

  const summaryRows = [
    { label: t.osName, value: metrics.identity.osName },
    { label: t.kernelName, value: metrics.identity.kernelName },
    { label: t.kernelVersion, value: metrics.identity.kernelVersion },
    { label: t.architecture, value: metrics.identity.architecture },
    { label: t.hostname, value: metrics.identity.hostname },
    { label: t.accessAddress, value: activeProfile?.host || activeSession.accessHost || '-' },
    { label: t.privateIp, value: metrics.ip || '-' },
    { label: t.running, value: metrics.uptime || '-' },
    { label: t.load, value: metrics.load || '-' }
  ]

  return (
    <section className="system-info-workspace">
      <header className="system-info-header">
        <div>
          <strong>{t.systemInfo}</strong>
          <p>{t.systemInfoDescription}</p>
        </div>
      </header>

      <div className="system-info-grid">
        <DataCard title={t.overview}>
          <DescriptionList rows={summaryRows} />
        </DataCard>

        <DataCard title={t.cpuDetails}>
          <Table
            columns={[t.model, t.cores, t.frequency, t.cache, t.bogomips]}
            rows={metrics.cpuInfoRows.map((row) => [
              row.model,
              String(row.cores || '-'),
              row.frequencyMHz === '-' ? '-' : `${row.frequencyMHz} MHz`,
              row.cache,
              row.bogomips
            ])}
          />
        </DataCard>

        <DataCard title={t.cpuUsage}>
          <Table
            columns={[t.user, t.system, t.nice, t.idle, t.ioWait, t.irq, t.softIrq, t.realtime]}
            rows={[[
              formatPercent(metrics.cpuUsage.user),
              formatPercent(metrics.cpuUsage.system),
              formatPercent(metrics.cpuUsage.nice),
              formatPercent(metrics.cpuUsage.idle),
              formatPercent(metrics.cpuUsage.ioWait),
              formatPercent(metrics.cpuUsage.irq),
              formatPercent(metrics.cpuUsage.softIrq),
              formatPercent(metrics.cpuUsage.steal)
            ]]}
          />
        </DataCard>

        <div className="system-info-split">
          <DataCard title={t.memoryUsageTitle}>
            <Table
              columns={[t.total, t.used, t.remaining, t.usage]}
              rows={[[
                metrics.memoryBreakdown.total,
                metrics.memoryBreakdown.used,
                metrics.memoryBreakdown.available,
                formatPercent(metrics.memoryBreakdown.percent)
              ]]}
            />
          </DataCard>
          <DataCard title={t.swapUsageTitle}>
            <Table
              columns={[t.total, t.used, t.remaining, t.usage]}
              rows={[[
                metrics.swapBreakdown.total,
                metrics.swapBreakdown.used,
                metrics.swapBreakdown.available,
                formatPercent(metrics.swapBreakdown.percent)
              ]]}
            />
          </DataCard>
        </div>

        <DataCard title={t.networkInterfaces}>
          <Table
            columns={[t.name, t.send, t.receive, t.sendRate, t.receiveRate]}
            rows={metrics.networkInterfaceRows.map((row) => [
              row.name,
              row.txTotal,
              row.rxTotal,
              row.txRate,
              row.rxRate
            ])}
          />
        </DataCard>

        <DataCard title={t.fileSystems}>
          <Table
            columns={[t.name, t.size, t.usage, t.available, t.mountPoint]}
            rows={metrics.fileSystemRows.map((row) => [
              row.name,
              `${row.used} / ${row.size}`,
              row.usagePercent,
              row.available,
              row.mountPoint
            ])}
          />
        </DataCard>
      </div>
    </section>
  )
}

function DataCard({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="system-info-card">
      <div className="system-info-card-head">
        <strong>{title}</strong>
      </div>
      {children}
    </section>
  )
}

function DescriptionList({ rows }: { rows: Array<{ label: string; value: string }> }) {
  return (
    <div className="system-description-list">
      {rows.map((row) => (
        <div className="system-description-row" key={row.label}>
          <span>{row.label}</span>
          <strong title={row.value}>{row.value}</strong>
        </div>
      ))}
    </div>
  )
}

function Table({
  columns,
  rows
}: {
  columns: string[]
  rows: string[][]
}) {
  return (
    <div className="system-table-shell">
      <table className="system-table">
        <thead>
          <tr>
            {columns.map((column) => <th key={column}>{column}</th>)}
          </tr>
        </thead>
        <tbody>
          {rows.length ? rows.map((row, rowIndex) => (
            <tr key={`${columns[0]}-${rowIndex}`}>
              {row.map((cell, cellIndex) => <td key={`${columns[cellIndex]}-${cellIndex}`}>{cell || '-'}</td>)}
            </tr>
          )) : (
            <tr>
              <td colSpan={columns.length}>-</td>
            </tr>
          )}
        </tbody>
      </table>
    </div>
  )
}

function formatPercent(value: number) {
  return `${value.toFixed(1)}%`
}
