import { Routes, Route } from 'react-router-dom'
import { lazy, Suspense, useEffect, useState } from 'react'
import { useAuthStore } from '@/stores/auth-store'
import Login from '@/pages/Login'
import { AppLayout } from '@/components/layout/AppLayout'

const Studio = lazy(() => import('@/pages/Studio'))
const FilesPage = lazy(() => import('@/pages/files/FilesPage'))
const AdminPage = lazy(() => import('@/pages/AdminPage'))

function Loading() {
  return <div className="flex items-center justify-center h-screen text-slate-400">加载中…</div>
}

/** 整页跳转回首页（/ 由 nginx 路由到门户 Dashboard） */
function RedirectHome() {
  useEffect(() => {
    window.location.href = '/'
  }, [])
  return <Loading />
}

function App() {
  const isAuthenticated = useAuthStore((s) => s.isAuthenticated)
  const logout = useAuthStore((s) => s.logout)
  const updateUser = useAuthStore((s) => s.updateUser)
  const [checking, setChecking] = useState(true)

  // 以服务器 Cookie 会话为准：localStorage 里可能残留旧 token（签名仍有效但无 Cookie），
  // 若只信 store 会在 /login ↔ / 之间与 nginx 门禁形成无限跳转。
  useEffect(() => {
    if (!isAuthenticated) {
      setChecking(false)
      return
    }
    // 不带 Authorization 头 → 服务端只校验 wa_session Cookie
    fetch('/api/auth/session-check')
      .then((resp) => {
        if (!resp.ok) {
          logout()
          return
        }
        // 会话有效：拉最新 user（昵称/头像登录后可能已更新，自愈旧缓存）
        fetch('/api/auth/session-token')
          .then((r2) => (r2.ok ? r2.json() : null))
          .then((data) => {
            if (data?.user) updateUser(data.user)
          })
          .catch(() => {})
      })
      .catch(() => logout())
      .finally(() => setChecking(false))
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  if (checking) return <Loading />

  if (!isAuthenticated) {
    return (
      <Routes>
        <Route path="/login" element={<Login />} />
        <Route path="/*" element={<Login />} />
      </Routes>
    )
  }

  return (
    <Suspense fallback={<Loading />}>
      <Routes>
        {/* 已登录访问 /login 或其它未知路径：整页跳首页 Dashboard（nginx 路由到门户） */}
        <Route path="/login" element={<RedirectHome />} />
        {/* 办公助手：部署在 /office 路径（首页由门户 Dashboard 项目接管） */}
        <Route path="/office" element={<Studio />} />
        <Route element={<AppLayout />}>
          <Route path="/" element={<RedirectHome />} />
          <Route path="/files" element={<FilesPage />} />
          <Route path="/admin" element={<AdminPage />} />
          <Route path="/*" element={<RedirectHome />} />
        </Route>
      </Routes>
    </Suspense>
  )
}

export default App
