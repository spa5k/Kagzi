import * as React from "react";
import { useListNamespaces } from "./use-grpc-services";
import type { Namespace } from "@/gen/namespace_pb";
import { ListNamespacesRequest } from "@/gen/namespace_pb";
import { PageRequest } from "@/gen/common_pb";

const NAMESPACE_STORAGE_KEY = "kagzi:namespace";

interface NamespaceContextValue {
  namespace: string;
  setNamespace: (namespace: string) => void;
  recentNamespaces: string[];
  namespaces: Namespace[];
  isLoadingNamespaces: boolean;
  errorNamespaces: unknown;
}

const NamespaceContext = React.createContext<NamespaceContextValue | null>(null);

function getStoredNamespace(): string {
  if (typeof window === "undefined") return "default";
  return localStorage.getItem(NAMESPACE_STORAGE_KEY) || "default";
}

function getRecentNamespaces(): string[] {
  if (typeof window === "undefined") return ["default"];
  const stored = localStorage.getItem(`${NAMESPACE_STORAGE_KEY}:recent`);
  if (!stored) return ["default"];
  try {
    return JSON.parse(stored);
  } catch {
    return ["default"];
  }
}

function saveRecentNamespaces(namespaces: string[]) {
  localStorage.setItem(`${NAMESPACE_STORAGE_KEY}:recent`, JSON.stringify(namespaces));
}

/**
 * Provider that manages the current namespace selection and persists it to localStorage.
 * Fetches namespaces from the backend and provides them to the app.
 * Wrap your app with this provider to enable namespace switching.
 */
export function NamespaceProvider({ children }: { children: React.ReactNode }) {
  const [namespace, setNamespaceState] = React.useState(getStoredNamespace);
  const [recentNamespaces, setRecentNamespaces] = React.useState(getRecentNamespaces);

  // Fetch namespaces from backend with proper pagination
  const request = React.useMemo(
    () =>
      new ListNamespacesRequest({
        page: new PageRequest({
          pageSize: 100,
          pageToken: "",
        }),
        includeDeleted: false,
      }),
    [],
  );
  const { data: namespacesData, isLoading, error } = useListNamespaces(request);

  const setNamespace = React.useCallback((newNamespace: string) => {
    const trimmed = newNamespace.trim();
    if (!trimmed) return;

    setNamespaceState(trimmed);
    localStorage.setItem(NAMESPACE_STORAGE_KEY, trimmed);

    // Update recent namespaces - keep unique, most recent first
    setRecentNamespaces((prev) => {
      const updated = [trimmed, ...prev.filter((n) => n !== trimmed)].slice(0, 10);
      saveRecentNamespaces(updated);
      return updated;
    });
  }, []);

  const namespaces = React.useMemo(() => namespacesData?.namespaces ?? [], [namespacesData]);

  const value = React.useMemo(
    () => ({
      namespace,
      setNamespace,
      recentNamespaces,
      namespaces,
      isLoadingNamespaces: isLoading,
      errorNamespaces: error,
    }),
    [namespace, setNamespace, recentNamespaces, namespaces, isLoading, error],
  );

  return <NamespaceContext.Provider value={value}>{children}</NamespaceContext.Provider>;
}

/**
 * Hook to get and set the current namespace.
 * Must be used within a NamespaceProvider.
 */
export function useNamespace(): NamespaceContextValue {
  const context = React.useContext(NamespaceContext);
  if (!context) {
    throw new Error("useNamespace must be used within a NamespaceProvider");
  }
  return context;
}
