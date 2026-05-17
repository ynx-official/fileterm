import { useEffect, useState } from 'react'
import type { ConnectionProfile, SessionSnapshot, SystemMetrics } from '@termdock/core'
import { copyText } from '../../app/app-utils'
import { t } from '../../i18n'

export function SystemSidebar({
  activeProfile,
  activeSession
}: {
  activeProfile: ConnectionProfile | null
  activeSession: SessionSnapshot | null
}) {
  const metrics = activeSession?.systemMetrics
  const internalIp = metrics?.ip || '-'
  const accessAddress = activeProfile?.host || activeSession?.accessHost || '-'
  const rows = activeSession?.systemMetrics?.diskRows ?? []

  return (
    <>
      <section className="sys-card">
        <div className="connection-summary">
          <AddressLine label={t.privateIp} value={internalIp} />
          <AddressLine label={t.accessAddress} value={accessAddress} />
        </div>
        <button className="system-title" type="button">{t.systemInfo}</button>
        <div className="metric-line"><span>{t.running}</span><strong>{metrics?.uptime ?? '-'}</strong></div>
        <div className="metric-line"><span>{t.load}</span><strong>{metrics?.load ?? '-'}</strong></div>
        <Meter label={t.cpu} value={metrics?.cpuPercent ?? 0} tone="green" caption={metrics ? `${metrics.cpuPercent}%` : '0%'} />
        <Meter label={t.memory} value={metrics?.memoryPercent ?? 0} tone="orange" caption={metrics?.memoryUsage ?? '0/0'} />
        <Meter label={t.swap} value={metrics?.swapPercent ?? 0} tone="yellow" caption={metrics?.swapUsage ?? '0/0'} />
        <div className="mini-tabs">
          <span>{t.memory}</span>
          <span>{t.cpu}</span>
          <span>{t.command}</span>
        </div>
        <ProcessTable rows={metrics?.topProcesses ?? []} />
        <NetworkPanel metrics={metrics} />
      </section>
      <section className="disk-table">
        <div className="disk-head"><span>{t.path}</span><span>{t.availableSize}</span></div>
        {rows.map((row) => (
          <div className="disk-row" key={row.path}><span>{row.path}</span><span>{row.usage}</span></div>
        ))}
      </section>
    </>
  )
}

function AddressLine({ label, value }: { label: string; value: string }) {
  const canCopy = value && value !== '-'

  return (
    <div className="address-row">
      <span>{label}</span>
      <strong title={value}>{value}</strong>
      <button
        className="copy-link"
        disabled={!canCopy}
        onClick={() => {
          if (canCopy) {
            copyText(value)
          }
        }}
        type="button"
      >
        复制
      </button>
    </div>
  )
}

function Meter({ label, value, tone, caption }: { label: string; value: number; tone: string; caption: string }) {
  return (
    <div className="meter-row">
      <span>{label}</span>
      <div className="meter-track"><i className={`meter-fill ${tone}`} style={{ width: `${value}%` }} /></div>
      <strong>{caption}</strong>
    </div>
  )
}

function ProcessTable({ rows }: { rows: SystemMetrics['topProcesses'] }) {
  return (
    <div className="process-table">
      {rows.length ? rows.map((row) => (
        <div className="process-row" key={`${row.command}-${row.memory}`}>
          <span>{row.memory}</span>
          <span>{row.cpu}</span>
          <span>{row.command}</span>
        </div>
      )) : <div className="process-empty" />}
    </div>
  )
}

function NetworkPanel({ metrics }: { metrics?: SystemMetrics }) {
  const [selectedInterface, setSelectedInterface] = useState(metrics?.activeNetworkInterface ?? '')

  useEffect(() => {
    setSelectedInterface(metrics?.activeNetworkInterface ?? '')
  }, [metrics?.activeNetworkInterface])

  return (
    <>
      <div className="network-panel">
        <div>
          <span className="up">↑{metrics?.networkRates.tx ?? '0B'}</span>
          <span className="down">↓{metrics?.networkRates.rx ?? '0B'}</span>
        </div>
        <select
          className="network-select"
          value={selectedInterface}
          onChange={(event) => setSelectedInterface(event.target.value)}
        >
          {(metrics?.networkInterfaces.length ? metrics.networkInterfaces : ['-']).map((name) => (
            <option key={name} value={name}>{name}</option>
          ))}
        </select>
      </div>
      <div className="grid-chart">
        {(metrics?.networkSamples.length ? metrics.networkSamples : Array.from({ length: 18 }, () => ({ rx: 0, tx: 0 }))).map((sample, index) => {
          const maxValue = Math.max(...(metrics?.networkSamples ?? [{ rx: 1, tx: 1 }]).flatMap((item) => [item.rx, item.tx]), 1)
          return (
            <div className="grid-chart-bar" key={`${selectedInterface}-${index}`}>
              <i className="tx-bar" style={{ height: `${Math.max(4, (sample.tx / maxValue) * 100)}%` }} />
              <i className="rx-bar" style={{ height: `${Math.max(4, (sample.rx / maxValue) * 100)}%` }} />
            </div>
          )
        })}
      </div>
    </>
  )
}
