import { useState, type FormEvent } from 'react'
import type { ConnectionFormMode, CreateProfileInput } from '@termdock/core'
import { t } from '../../i18n'

export function ConnectionModal({
  errorMessage,
  mode,
  form,
  setForm,
  onSubmit,
  onClose,
  standalone = false
}: {
  errorMessage: string | null
  mode: ConnectionFormMode
  form: CreateProfileInput
  setForm(value: CreateProfileInput | ((prev: CreateProfileInput) => CreateProfileInput)): void
  onSubmit(event: FormEvent<HTMLFormElement>): void
  onClose(): void
  standalone?: boolean
}) {
  const [section, setSection] = useState<'ssh' | 'terminal' | 'proxy' | 'tunnel'>('ssh')

  const content = (
    <div className={`modal-card ssh-modal ${standalone ? 'standalone' : ''}`}>
      <div className="modal-header">
        <span>{mode === 'edit' ? '编辑连接' : t.newConnection}</span>
        {!standalone ? <button className="icon-button" onClick={onClose} type="button">×</button> : null}
      </div>
      <div className="ssh-modal-body">
        <aside className="ssh-modal-nav">
          <button className={section === 'ssh' ? 'active' : ''} type="button" onClick={() => setSection('ssh')}>SSH连接</button>
          <button className={section === 'terminal' ? 'active' : ''} type="button" onClick={() => setSection('terminal')}>终端</button>
          <button className={section === 'proxy' ? 'active' : ''} type="button" onClick={() => setSection('proxy')}>代理服务器</button>
          <button className={section === 'tunnel' ? 'active' : ''} type="button" onClick={() => setSection('tunnel')}>隧道</button>
        </aside>
        <form className="ssh-form-shell" onSubmit={onSubmit}>
          {section === 'ssh' ? (
            <div className="ssh-form-page">
              <fieldset className="ssh-fieldset">
                <legend>常规</legend>
                <div className="ssh-grid ssh-grid-general">
                  <label>类型:
                    <select
                      value={form.type}
                      onChange={(event) => {
                        const nextType = event.target.value as 'ssh' | 'ftp'
                        setForm((prev) => ({
                          ...prev,
                          type: nextType,
                          port: nextType === 'ftp' && prev.port === 22 ? 21 : nextType === 'ssh' && prev.port === 21 ? 22 : prev.port,
                          authType: nextType === 'ssh' ? prev.authType ?? 'password' : 'password'
                        }))
                      }}
                    >
                      <option value="ssh">SSH / SFTP</option>
                      <option value="ftp">FTP / FTPS</option>
                    </select>
                  </label>
                  <label>分组:<input value={form.group} onChange={(event) => setForm((prev) => ({ ...prev, group: event.target.value }))} /></label>
                  <label className="span-2">名称:<input value={form.name} onChange={(event) => setForm((prev) => ({ ...prev, name: event.target.value }))} /></label>
                  <label className="span-2">主机:<input value={form.host} onChange={(event) => setForm((prev) => ({ ...prev, host: event.target.value }))} /></label>
                  <label className="narrow">端口:<input inputMode="numeric" value={form.port || ''} onChange={(event) => setForm((prev) => ({ ...prev, port: Number(event.target.value.replace(/\D/g, '')) }))} /></label>
                  <label>远程路径:<input value={form.remotePath} onChange={(event) => setForm((prev) => ({ ...prev, remotePath: event.target.value }))} /></label>
                  <label className="full">备注:<textarea value={form.note ?? ''} onChange={(event) => setForm((prev) => ({ ...prev, note: event.target.value }))} /></label>
                </div>
              </fieldset>
              <fieldset className="ssh-fieldset">
                <legend>认证</legend>
                <div className="ssh-grid ssh-grid-auth">
                  {form.type === 'ssh' ? (
                    <label>方法:
                      <select value={form.authType} onChange={(event) => setForm((prev) => ({ ...prev, authType: event.target.value as 'password' | 'privateKey' }))}>
                        <option value="password">密码</option>
                        <option value="privateKey">私钥</option>
                      </select>
                    </label>
                  ) : null}
                  <label>用户名:<input value={form.username} onChange={(event) => setForm((prev) => ({ ...prev, username: event.target.value }))} /></label>
                  <label className="span-2">密码:<input type="password" value={form.password ?? ''} onChange={(event) => setForm((prev) => ({ ...prev, password: event.target.value }))} /></label>
                  {form.type === 'ssh' ? (
                    <label className="full">私钥:<input value={form.privateKeyPath ?? ''} onChange={(event) => setForm((prev) => ({ ...prev, privateKeyPath: event.target.value }))} /></label>
                  ) : (
                    <label className="ssh-checkbox span-2">
                      <input checked={Boolean(form.secure)} type="checkbox" onChange={(event) => setForm((prev) => ({ ...prev, secure: event.target.checked }))} />
                      <span>使用 FTPS</span>
                    </label>
                  )}
                </div>
              </fieldset>
              {form.type === 'ssh' ? <fieldset className="ssh-fieldset">
                <legend>高级</legend>
                <label className="ssh-checkbox">
                  <input checked={Boolean(form.enableExecChannel)} type="checkbox" onChange={(event) => setForm((prev) => ({ ...prev, enableExecChannel: event.target.checked }))} />
                  <span>启用Exec Channel(若连接上就被断开,请关闭该项,比如跳板机)</span>
                </label>
              </fieldset> : null}
            </div>
          ) : null}
          {section === 'terminal' ? (
            <div className="ssh-form-page">
              <fieldset className="ssh-fieldset narrow">
                <legend>终端</legend>
                <div className="ssh-grid single">
                  <label>字符编码:
                    <select value={form.encoding} onChange={(event) => setForm((prev) => ({ ...prev, encoding: event.target.value }))}>
                      <option value="UTF-8">UTF-8</option>
                      <option value="GBK">GBK</option>
                    </select>
                  </label>
                  <div className="terminal-key-box">
                    <strong>按键序列(解决退格/删除键失效,乱码问题):</strong>
                    <label>Backspace退格键
                      <select value={form.backspaceKey} onChange={(event) => setForm((prev) => ({ ...prev, backspaceKey: event.target.value }))}>
                        <option value="ASCII">ASCII - Backspace</option>
                        <option value="DEL">DEL - Backspace</option>
                      </select>
                    </label>
                    <label>Delete删除键
                      <select value={form.deleteKey} onChange={(event) => setForm((prev) => ({ ...prev, deleteKey: event.target.value }))}>
                        <option value="VT220">VT220 - Delete</option>
                        <option value="ASCII">ASCII - Delete</option>
                      </select>
                    </label>
                  </div>
                </div>
              </fieldset>
            </div>
          ) : null}
          {section === 'proxy' ? <div className="ssh-placeholder">代理服务器功能稍后接入</div> : null}
          {section === 'tunnel' ? <div className="ssh-placeholder">隧道功能稍后接入</div> : null}
          {errorMessage ? <div className="modal-error">{errorMessage}</div> : null}
          <div className="form-actions ssh-actions">
            <button className="flat-button" onClick={onClose} type="button">{t.cancel}</button>
            <button className="primary-button" type="submit">{mode === 'edit' ? '保存修改' : t.saveConnection}</button>
          </div>
        </form>
      </div>
    </div>
  )

  if (standalone) {
    return <div className="connection-form-window">{content}</div>
  }

  return (
    <div className="modal-backdrop">
      {content}
    </div>
  )
}
