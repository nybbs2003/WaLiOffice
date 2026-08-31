import { Routes, Route, Navigate } from 'react-router-dom'
import { lazy, Suspense } from 'react'
import { useAuthStore } from '@/stores/auth-store'
import Login from '@/pages/Login'
import { AppLayout } from '@/components/layout/AppLayout'

const Studio = lazy(() => import('@/pages/Studio'))
const FilesPage = lazy(() => import('@/pages/files/FilesPage'))
const AdminPage = lazy(() => import('@/pages/AdminPage'))

function Loading() {
  return <div className="flex items-center justify-center h-screen text-slate-400">加载中…</div>
}

function App() {
  const isAuthenticated = useAuthStore((s) => s.isAuthenticated)

  if (!isAuthenticated) {
    return (
      <Routes>
        <Route path="/login" element={<Login />} />
        <Route path="/*" element={<Navigate to="/login" />} />
      </Routes>
    )
  }

  return (
    <Suspense fallback={<Loading />}>
      <Routes>
        <Route path="/login" element={<Navigate to="/office" />} />
        {/* 办公助手：部署在 /office 路径（首页由门户 Dashboard 项目接管） */}
        <Route path="/office" element={<Studio />} />
        <Route element={<AppLayout />}>
          <Route path="/" element={<Navigate to="/office" />} />
          <Route path="/files" element={<FilesPage />} />
          <Route path="/admin" element={<AdminPage />} />
          <Route path="/*" element={<Navigate to="/office" />} />
        </Route>
      </Routes>
    </Suspense>
  )
}

export default App
