import { useEffect, useMemo, useState } from 'react'
import type {
  ConnectionFolder,
  ConnectionFormMode,
  ConnectionProfile,
  CreateProfileInput,
  FileTermDesktopApi
} from '@fileterm/core'
import { defaultForm, profileToForm } from '../app/app-data'
import { t } from '../i18n'

export type WindowCloseConfirmState = {
  isQuit: boolean
  hasActiveConnections: boolean
}

type UseWorkspaceModalsOptions = {
  desktopApi?: FileTermDesktopApi
  folders: ConnectionFolder[]
  formWindowMode: ConnectionFormMode
  formWindowProfileId: string | null
  hasLoadedInitialSnapshot: boolean
  isConnectionFormWindow: boolean
  /**
   * 当前窗口是否为可拆分会话独立窗口。detached-session 窗口关闭时
   * 走 `close-detached` action（destroy + 标签归还 main），而非主窗口
   * 的 `hide` action（隐藏到托盘）。
   */
  isDetachedSessionWindow: boolean
  profiles: ConnectionProfile[]
}

function collectConnectionGroups(folderNames: string[], profileGroups: string[], currentGroup?: string) {
  const groups = new Set<string>(['默认'])

  for (const name of folderNames) {
    const value = name.trim()
    if (value) {
      groups.add(value)
    }
  }

  for (const group of profileGroups) {
    const value = group.trim()
    if (value) {
      groups.add(value)
    }
  }

  if (currentGroup?.trim()) {
    groups.add(currentGroup.trim())
  }

  return [...groups]
}

export function useWorkspaceModals({
  desktopApi,
  folders,
  formWindowMode,
  formWindowProfileId,
  hasLoadedInitialSnapshot,
  isConnectionFormWindow,
  isDetachedSessionWindow,
  profiles
}: UseWorkspaceModalsOptions) {
  const [showConnectionForm, setShowConnectionForm] = useState(false)
  const [showConnectionManager, setShowConnectionManager] = useState(false)
  const [showCommandManager, setShowCommandManager] = useState(false)
  const [showSettings, setShowSettings] = useState(false)
  const [form, setForm] = useState<CreateProfileInput>(defaultForm)
  const [formError, setFormError] = useState<string | null>(null)
  const [editingProfileId, setEditingProfileId] = useState<string | null>(null)
  const [windowCloseConfirm, setWindowCloseConfirm] = useState<WindowCloseConfirmState | null>(null)

  const connectionGroupOptions = useMemo(
    () =>
      collectConnectionGroups(
        folders.map((folder) => folder.name),
        profiles.map((profile) => profile.group),
        form.group
      ),
    [folders, form.group, profiles]
  )

  useEffect(() => {
    if (!isConnectionFormWindow) {
      return
    }

    if (formWindowMode === 'edit') {
      const profile = profiles.find((item) => item.id === formWindowProfileId)
      if (!profile) {
        // A standalone edit window renders once before its workspace snapshot
        // arrives. Do not briefly show a false "profile not found" error.
        if (hasLoadedInitialSnapshot) {
          setFormError(t.profileNotFound)
        }
        return
      }

      setEditingProfileId(profile.id)
      setForm(profileToForm(profile))
      setFormError(null)
      return
    }

    setEditingProfileId(null)
    setForm(defaultForm)
    setFormError(null)
  }, [formWindowMode, formWindowProfileId, hasLoadedInitialSnapshot, isConnectionFormWindow, profiles])

  const updateForm = (updater: CreateProfileInput | ((current: CreateProfileInput) => CreateProfileInput)) => {
    setForm((current) => (typeof updater === 'function' ? updater(current) : updater))
    setFormError(null)
  }

  const openCreateModal = () => {
    setEditingProfileId(null)
    setForm(defaultForm)
    setFormError(null)
    setShowConnectionForm(true)
  }

  const openEditModal = (profile: ConnectionProfile) => {
    setEditingProfileId(profile.id)
    setForm(profileToForm(profile))
    setFormError(null)
    setShowConnectionForm(true)
  }

  const openCreateConnection = () => {
    if (desktopApi) {
      void desktopApi.openConnectionFormWindow('create')
      return
    }
    openCreateModal()
  }

  const openEditConnection = (profile: ConnectionProfile) => {
    if (desktopApi) {
      return desktopApi.openConnectionFormWindow('edit', profile.id)
    }
    openEditModal(profile)
  }

  const closeConnectionForm = () => {
    setShowConnectionForm(false)
    setEditingProfileId(null)
    setFormError(null)
  }

  const openConnectionManager = () => {
    if (desktopApi) {
      void desktopApi.openConnectionManagerWindow()
      return
    }
    setShowConnectionManager(true)
  }

  const openCommandManager = () => {
    if (desktopApi) {
      void desktopApi.openCommandManagerWindow()
      return
    }
    setShowCommandManager(true)
  }

  const requestWindowCloseConfirmation = (isQuit: boolean, hasActiveConnections: boolean) => {
    if (desktopApi?.platform === 'darwin' && !isQuit && !hasActiveConnections) {
      // detached-session 窗口无活动连接时直接销毁（标签归还 main）；
      // 主窗口才走 hide（隐藏到托盘）。
      const action = isDetachedSessionWindow ? 'close-detached' : 'hide'
      void desktopApi.confirmCloseWindow(action)
      return
    }

    setWindowCloseConfirm({ isQuit, hasActiveConnections })
  }

  const resolveWindowCloseConfirmation = (action: 'quit' | 'hide' | 'cancel') => {
    setWindowCloseConfirm(null)
    // detached-session 窗口没有"隐藏到托盘"语义：确认关闭即销毁窗口，
    // 标签 owner 归还 main。'quit' 仍走全局退出流程。
    if (isDetachedSessionWindow && action === 'hide') {
      void desktopApi?.confirmCloseWindow('close-detached')
      return
    }
    void desktopApi?.confirmCloseWindow(action)
  }

  const openCommandManagerFromSettings = () => {
    setShowSettings(false)
    openCommandManager()
  }

  const openConnectionManagerFromSettings = () => {
    setShowSettings(false)
    openConnectionManager()
  }

  return {
    closeConnectionForm,
    connectionGroupOptions,
    editingProfileId,
    form,
    formError,
    openCommandManager,
    openCommandManagerFromSettings,
    openConnectionManager,
    openConnectionManagerFromSettings,
    openCreateConnection,
    openCreateModal,
    openEditConnection,
    openEditModal,
    requestWindowCloseConfirmation,
    resolveWindowCloseConfirmation,
    setForm,
    setFormError,
    setShowCommandManager,
    setShowConnectionManager,
    setShowSettings,
    showCommandManager,
    showConnectionForm,
    showConnectionManager,
    showSettings,
    updateForm,
    windowCloseConfirm
  }
}
