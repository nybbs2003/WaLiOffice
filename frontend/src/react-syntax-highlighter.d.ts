declare module 'react-syntax-highlighter' {
  import type { ComponentType, ReactNode } from 'react'
  export const Prism: ComponentType<{
    language?: string
    style?: Record<string, any>
    customStyle?: Record<string, any>
    codeTagProps?: Record<string, any>
    wrapLongLines?: boolean
    children: ReactNode
  }>
  const Light: ComponentType<any>
  export default Light
}

declare module 'react-syntax-highlighter/dist/esm/styles/prism' {
  export const oneLight: Record<string, any>
  export const oneDark: Record<string, any>
  export const vscDarkPlus: Record<string, any>
}
