import { useEffect, useState } from 'react'
import { useAuthStore } from '@/stores/auth-store'
import { authApi } from '@/api'
import { Loader2 } from 'lucide-react'

const LOGO_URL = '/logo.png'

export default function Login() {
  const login = useAuthStore((s) => s.login)

  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')

  // 飞书登录
  const [feishuEnabled, setFeishuEnabled] = useState(false)
  const [feishuAppId, setFeishuAppId] = useState('')
  const [feishuRedirect, setFeishuRedirect] = useState('')

  useEffect(() => {
    authApi.feishuConfig().then(({ data }) => {
      if (data.enabled) {
        setFeishuEnabled(true)
        setFeishuAppId(data.app_id)
        setFeishuRedirect(data.redirect_uri)
      }
    }).catch(() => {})

    // nginx 门禁未登录时携带 next 跳转过来：自动拉起飞书登录
    const params = new URLSearchParams(window.location.search)
    const code = params.get('code')
    const next = params.get('next')
    if (code) {
      handleFeishuCallback(code)
    } else {
      // 门户 SSO：浏览器已带 wa_session Cookie（例如先登录过门户/Office）时自动登录
      // 只带 Cookie 请求（不附加 localStorage 里的旧 Bearer，避免登出后又被旧 token 拉回登录态）
      fetch('/api/auth/session-token')
        .then((resp) => (resp.ok ? resp.json() : Promise.reject(new Error('no session'))))
        .then((data) => {
          login(data.access_token, data.user)
          afterLoginNavigate()
        })
        .catch(() => {})
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // feishu 配置就绪且带 next 时，自动拉起飞书授权（避免多一次点击）
  useEffect(() => {
    const params = new URLSearchParams(window.location.search)
    const code = params.get('code')
    const next = params.get('next')
    if (!code && next && feishuEnabled && feishuAppId && !loading) {
      handleFeishuLogin()
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [feishuEnabled, feishuAppId])

  const afterLoginNavigate = () => {
    // 登录后统一整页跳转首页（/ 由 nginx 路由到门户 Dashboard）
    window.location.href = '/'
  }

  const handleFeishuCallback = async (code: string) => {
    setLoading(true)
    setError('')
    try {
      const { data } = await authApi.feishuLogin(code)
      login(data.access_token, data.user)
      afterLoginNavigate()
    } catch (err: any) {
      setError(err.response?.data?.detail || '飞书登录失败')
    } finally {
      setLoading(false)
    }
  }

  const handleFeishuLogin = () => {
    const redirect = feishuRedirect || `${window.location.origin}/login`
    // 带 offline_access 以获取 refresh_token + 基础用户信息 scope
    const scope = 'offline_access auth:user.id:read contact:user.base:readonly docx:document:readonly bitable:app:readonly calendar:calendar:read drive:drive:readonly wiki:wiki:readonly'
    const url = `https://open.feishu.cn/open-apis/authen/v1/authorize?app_id=${feishuAppId}&redirect_uri=${encodeURIComponent(redirect)}&scope=${encodeURIComponent(scope)}&state=${Date.now()}`
    window.location.href = url
  }

  return (
    <div className="min-h-screen bg-white px-4 py-8 text-surface-900">
      <main className="mx-auto flex min-h-[calc(100vh-4rem)] w-full max-w-xl flex-col items-center justify-center text-center">
        <div className="mb-5 flex h-16 w-16 items-center justify-center overflow-hidden rounded-2xl bg-white shadow-sm ring-1 ring-black/[0.06]">
          <img src={LOGO_URL} alt="Moe Office logo" className="h-full w-full object-cover" />
        </div>

        <h1 className="text-4xl font-extrabold tracking-tight text-surface-900">Moe Office</h1>
        <p className="mt-4 text-lg font-semibold text-surface-700">学习AI办公、掌握AI部署、运用AI提效</p>

        <div className="mt-10 w-full max-w-sm space-y-4">
          {/* 飞书登录（唯一登录方式） */}
          {feishuEnabled ? (
            <button
              type="button"
              onClick={handleFeishuLogin}
              disabled={loading}
              className="flex h-12 w-full items-center justify-center gap-2 rounded-xl bg-[#3370ff] px-6 text-base font-semibold text-white transition hover:bg-[#2b5fd6] disabled:opacity-60"
            >
              {loading ? <Loader2 className="mr-2 h-5 w-5 animate-spin" /> : null}
              飞书账号登录
            </button>
          ) : (
            <div className="flex items-center justify-center gap-2 text-sm text-surface-500">
              <Loader2 className="h-4 w-4 animate-spin" />
              登录服务加载中…
            </div>
          )}

          {error && <div className="rounded-lg bg-red-50 px-4 py-2 text-sm text-red-600">{error}</div>}
        </div>

        <div className="mt-12 w-full max-w-lg text-left text-[13px] font-medium leading-6 text-surface-500">
          <p>
            Moe Office 基于开源项目
            <a href="https://github.com/fuzhengwei/WaLiOffice" target="_blank" rel="noreferrer" className="mx-1 text-primary-600 underline underline-offset-2">WaLiOffice</a>
            二次开发，并遵循其 MIT 开源协议（MIT License）。
          </p>
          <p className="mt-2">
            WaLiOffice 由
            <a href="https://bugstack.cn/" target="_blank" rel="noreferrer" className="mx-1 text-primary-600 underline underline-offset-2">小傅哥（fuzhengwei）</a>
            开发并开源。MIT 协议允许任何人在保留版权声明与许可声明的前提下，自由地使用、复制、修改、合并、发布甚至销售本软件的副本，无论是否用于商业用途，软件均按「现状」提供，作者不承担任何担保责任。
          </p>
          <p className="mt-2">
            本平台在 WaLiOffice 原有能力的基础上，进行了界面品牌、飞书登录、本地算力网关、多语言国际化、生图生视频适配等定制与扩展，并继续以开源精神共享。特此向原作者小傅哥与开源社区致谢。
          </p>
          <p className="mt-2">
            使用本平台请合理、合法、合规，输入与生成的内容请遵守当地法律法规。
          </p>
        </div>
      </main>
    </div>
  )
}
