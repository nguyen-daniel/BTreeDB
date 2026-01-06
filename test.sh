#!/bin/bash
set -e

echo "🔨 Building project..."
cargo build

echo ""
echo "🧪 Running tests..."
cargo test

echo ""
echo "📝 Checking code formatting..."
if ! cargo fmt --check; then
    echo "❌ Formatting check failed. Run 'cargo fmt' to fix formatting issues."
    exit 1
fi

echo ""
echo "🔍 Running clippy linter..."
cargo clippy -- -D warnings

echo ""
echo "🚀 Building release version..."
cargo build --release

echo ""
echo "✅ All checks passed! Ready to push."
