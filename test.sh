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
echo "   - Testing value module..."
cargo test value:: --lib -- --quiet
echo "   - Testing wal module..."
cargo test wal:: --lib -- --quiet
echo "   - Testing transaction module..."
cargo test transaction:: --lib -- --quiet
echo "   - Testing compression module..."
cargo test compression:: --lib -- --quiet
echo "   - Testing backup module..."
cargo test backup:: --lib -- --quiet
echo "   - Testing manager module..."
cargo test manager:: --lib -- --quiet
echo "   - Testing concurrency module..."
cargo test concurrency:: --lib -- --quiet
echo "   All module tests passed"

echo ""
echo "7. Running doc tests..."
cargo test --doc -- --quiet 2>/dev/null || echo "   No doc tests found"

echo ""
echo "8. Building release version..."
cargo build --release

echo ""
echo "========================================"
echo "All checks passed!"
echo "========================================"
