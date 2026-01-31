# Justfile for beads-lite

# Run tests
test:
    cargo test

# Run tests with verbose output
test-v:
    cargo test -- --nocapture

# Build release binary
build:
    cargo build --release
    cp target/release/bl ./bl

# Test then build
dev: test build

# Clean build artifacts
clean:
    cargo clean
    rm -f ./bl

# Format code
fmt:
    cargo fmt

# Run clippy lints
lint:
    cargo clippy -- -W clippy::all
