import { createRouter, RouterProvider } from "@tanstack/react-router";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { routeTree } from "./routeTree.gen";

import { TransportProvider } from "@connectrpc/connect-query";
import { QueryClientProvider } from "@tanstack/react-query";
import { NamespaceProvider } from "./hooks/use-namespace";
import "./index.css";
import { getGrpcTransport } from "./lib/grpc-client";
import { createQueryClient } from "./lib/query-config";

const queryClient = createQueryClient();
const grpcTransport = getGrpcTransport();

// Create a new router instance
const router = createRouter({
  routeTree,
  context: {
    queryClient,
  },
  defaultPreload: "intent",
  // Since we're using React Query, we don't want loader calls to ever be stale
  // This will ensure that the loader is always called when the route is preloaded or visited
  defaultPreloadStaleTime: 0,
  scrollRestoration: true,
});

// Register the router instance for type safety
declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

// Render the app
const rootElement = document.getElementById("root")!;
if (!rootElement.innerHTML) {
  const root = createRoot(rootElement);
  root.render(
    <StrictMode>
      <TransportProvider transport={grpcTransport}>
        <QueryClientProvider client={queryClient}>
          <NamespaceProvider>
            <RouterProvider router={router} />
          </NamespaceProvider>
        </QueryClientProvider>
      </TransportProvider>
    </StrictMode>,
  );
}
