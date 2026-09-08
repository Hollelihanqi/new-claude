import { createContext, startTransition, Suspense, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import PageErrorBoundary from "./PageErrorBoundary";

const PageActiveContext = createContext(true);
export const usePageActive = () => useContext(PageActiveContext);

// 首次请求由页面负责；再次显示时静默更新，不重置本地编辑状态。
export function usePageActivation(refresh: () => void, activeOverride?: boolean) {
  const contextActive = usePageActive();
  const active = activeOverride ?? contextActive;
  const previous = useRef(active);
  const callback = useRef(refresh);
  callback.current = refresh;
  useEffect(() => {
    if (active && !previous.current) callback.current();
    previous.current = active;
  }, [active]);
}

// 分批预热页面及其只读数据；切换只改变可见性，不销毁表单和请求状态。
export default function PersistentPage({ active, warmupDelay, children }: {
  active: boolean;
  warmupDelay: number;
  children: ReactNode;
}) {
  const [ready, setReady] = useState(active);
  useEffect(() => {
    if (active) setReady(true);
  }, [active]);
  useEffect(() => {
    const timer = setTimeout(() => startTransition(() => setReady(true)), warmupDelay);
    return () => clearTimeout(timer);
  }, [warmupDelay]);
  if (!active && !ready) return null;
  return (
    <div hidden={!active} style={{ display: active ? "contents" : "none" }}>
      <PageActiveContext.Provider value={active}>
        <PageErrorBoundary>
          <Suspense fallback={<div role="status" style={{ padding: 24 }}>正在准备页面…</div>}>
            {children}
          </Suspense>
        </PageErrorBoundary>
      </PageActiveContext.Provider>
    </div>
  );
}
