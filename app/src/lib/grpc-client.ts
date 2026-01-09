import type { Interceptor, Transport } from "@connectrpc/connect";
import { createGrpcWebTransport } from "@connectrpc/connect-web";

/**
 * Get the gRPC server URL from environment or use default
 */
function getServerUrl(): string {
  return import.meta.env.VITE_GRPC_URL || "http://localhost:50051";
}

/**
 * Logging interceptor for debugging gRPC requests
 */
const loggingInterceptor: Interceptor = (next) => async (req) => {
  console.log(`[gRPC] ${req.method.name}`, {
    method: req.method.name,
  });

  try {
    const response = await next(req);
    console.log(`[gRPC] ${req.method.name} - Success`);
    return response;
  } catch (error) {
    console.error(`[gRPC] ${req.method.name} - Error`, error);
    throw error;
  }
};

/**
 * Error handling interceptor
 */
const errorInterceptor: Interceptor = (next) => async (req) => {
  try {
    return await next(req);
  } catch (error) {
    // You can add custom error handling here
    // e.g., toast notifications, error tracking, etc.
    throw error;
  }
};

/**
 * Create and configure the gRPC-Web transport
 */
export function createGrpcTransport(): Transport {
  const baseUrl = getServerUrl();

  return createGrpcWebTransport({
    baseUrl,
    // Add interceptors for logging and error handling
    interceptors: [loggingInterceptor, errorInterceptor],
    // Configure default timeout (30 seconds)
    defaultTimeoutMs: 30000,
  });
}

/**
 * Singleton transport instance
 */
let transport: Transport | null = null;

/**
 * Get or create the transport instance
 */
export function getGrpcTransport(): Transport {
  if (!transport) {
    transport = createGrpcTransport();
  }
  return transport;
}

/**
 * Reset the transport (useful for testing or reconnecting)
 */
export function resetGrpcTransport(): void {
  transport = null;
}
