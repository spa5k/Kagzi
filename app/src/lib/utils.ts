import { type ClassValue, clsx } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export function decodeJsonBytes<T = unknown>(bytes: Uint8Array | undefined): T | undefined {
  if (!bytes || bytes.length === 0) return undefined;
  try {
    const text = new TextDecoder().decode(bytes);
    return JSON.parse(text) as T;
  } catch {
    return undefined;
  }
}
