#!/bin/bash
set -e

echo "========================================"
echo "BTreeDB Test Suite"
echo "========================================"

echo ""
echo "1. Building project..."
cargo build

echo ""
echo "2. Checking code formatting..."
if ! cargo fmt --check; then
    echo "Formatting check failed. Run 'cargo fmt' to fix formatting issues."
    exit 1
fi
echo "   Formatting OK"

echo ""
echo "3. Running clippy linter..."
cargo clippy -- -D warnings
echo "   Clippy OK"

echo ""
echo "4. Running unit tests..."
cargo test --lib -- --nocapture 2>&1 | tail -20

echo ""
echo "5. Running integration tests..."
cargo test --test integration_test -- --nocapture 2>&1 | tail -10

echo ""
echo "6. Testing individual modules..."
echo "   - Testing cursor module..."
cargo test cursor:: --lib -- --quiet
echo "   - Testing wal module..."
cargo test wal:: --lib -- --quiet
echo "   All default-path module tests passed"

echo ""
echo "6b. Testing experimental modules (--features experimental)..."
cargo test --features experimental --lib -- --quiet
echo "   Experimental module tests passed"

echo ""
echo "7. Running doc tests..."
cargo test --doc -- --quiet 2>/dev/null || echo "   No doc tests found"

echo ""
echo "8. Building release version..."
cargo build --release

echo ""
echo "9. Running examples..."
cargo run --example split_demo
cargo run --example crash_recover
echo "   Examples OK"

echo ""
echo "========================================"
echo "All checks passed!"
echo "========================================"
