import { createFileRoute, redirect } from "@tanstack/react-router";

const NAMESPACE_STORAGE_KEY = "kagzi:namespace";

function getStoredNamespace(): string {
  if (typeof window === "undefined") return "default";
  return localStorage.getItem(NAMESPACE_STORAGE_KEY) || "default";
}

export const Route = createFileRoute("/")({
  beforeLoad: () => {
    const namespaceId = getStoredNamespace();
    throw redirect({
      to: "/$namespaceId",
      params: { namespaceId },
    });
  },
});
