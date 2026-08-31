import { Routes, Route } from 'react-router-dom'
import { lazy, Suspense, useEffect } from 'react'
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
