import { create } from 'zustand';
import { persist } from 'zustand/middleware';
import type { User } from '@/types';

interface AuthState {
  token: string | null;
  user: User | null;
  isAuthenticated: boolean;
  login: (token: string, user: User) => void;
  logout: () => void;
  updateUser: (user: User) => void;
}

export const useAuthStore = create<AuthState>()(
  persist(
    (set) => ({
      token: null,
      user: null,
      isAuthenticated: false,
      login: (token, user) => set({ token, user, isAuthenticated: true }),
      logout: () => set({ token: null, user: null, isAuthenticated: false }),
      updateUser: (user) => set({ user }),
    }),
    {
      name: 'aippt-auth',
      // 只持久化 token + user；isAuthenticated 由 token 派生，避免状态不一致
      partialize: (state) => ({
        token: state.token,
        user: state.user,
      }),
      // 水合时用 token 是否存在派生 isAuthenticated
      merge: (persisted, current) => {
        const p = persisted as Partial<AuthState> | undefined;
        const token = p?.token ?? null;
        return {
          ...current,
          token,
          user: p?.user ?? null,
          isAuthenticated: !!token,
        };
      },
    }
  )
);
