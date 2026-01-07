#!/bin/bash
set -e

cd "$(dirname "$0")/.."

echo "🔄 Generating protobuf files..."

rm -rf src/gen
mkdir -p src/gen

./node_modules/.bin/buf \
  generate \
  --template buf.gen.yaml

echo "✅ Protobuf files generated"

