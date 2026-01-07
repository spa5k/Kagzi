#!/bin/bash
set -e

cd "$(dirname "$0")/.."

echo "🔄 Generating protobuf files..."

# Clean and create output directory
rm -rf src/gen
mkdir -p src/gen

# Run buf generate
if ! npx @bufbuild/buf generate; then
  echo "❌ Failed to generate protobuf files"
  exit 1
fi

echo "✅ Protobuf files generated successfully"

