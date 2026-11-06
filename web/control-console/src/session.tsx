import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";
import { type AccountView, ApiError, del, get, whenSignedOut } from "./api";

interface Session {
  /** `undefined` while loading, `null` when signed out. */
  me: AccountView | null | undefined;
  refresh: () => Promise<void>;
  signOut: () => Promise<void>;
}

const SessionContext = createContext<Session | null>(null);

export function SessionProvider({ children }: { children: ReactNode }) {
  const [me, setMe] = useState<AccountView | null | undefined>(undefined);
  const refresh = useCallback(async () => {
    try {
      setMe(await get<AccountView>("/v1/accounts/me"));
    } catch (error) {
      if (error instanceof ApiError && error.status === 401) {
        setMe(null);
      } else {
        throw error;
      }
    }
  }, []);
  const signOut = useCallback(async () => {
    try {
      await del("/v1/web/session");
    } finally {
      setMe(null);
    }
  }, []);
  useEffect(() => {
    whenSignedOut(() => setMe(null));
    void refresh().catch(() => setMe(null));
  }, [refresh]);
  return (
    <SessionContext.Provider value={{ me, refresh, signOut }}>{children}</SessionContext.Provider>
  );
}

export function useSession(): Session {
  const session = useContext(SessionContext);
  if (!session) {
    throw new Error("useSession outside SessionProvider");
  }
  return session;
}
