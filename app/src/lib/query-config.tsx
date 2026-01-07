import { TransportProvider } from "@connectrpc/connect-query";
import { QueryClient } from "@tanstack/react-query";
import { getGrpcTransport } from "./grpc-client";

/**
 * Create and configure the TanStack Query client
 */
export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        // Data is considered fresh for 5 seconds
        staleTime: 5000,
        // Keep unused data in cache for 5 minutes
        gcTime: 5 * 60 * 1000,
        // Retry failed requests up to 3 times
        retry: 3,
        // Exponential backoff for retries
        retryDelay: (attemptIndex) => Math.min(1000 * 2 ** attemptIndex, 30000),
        // Refetch on window focus for important data
        refetchOnWindowFocus: true,
        // Refetch on reconnect
        refetchOnReconnect: true,
      },
      mutations: {
        // Retry mutations once on failure
        retry: 1,
      },
    },
  });
}

/**
 * Get the gRPC transport for Connect Query
 */
export function getTransport() {
  return getGrpcTransport();
}

/**
 * Create the TransportProvider component with configured transport
 */
export function createTransportProvider() {
  const transport = getTransport();

  return function GrpcTransportProvider({ children }: { children: React.ReactNode }) {
    return <TransportProvider transport={transport}>{children}</TransportProvider>;
  };
}
