#!/bin/bash
set -e

# Setup temp dir
TEST_DIR=$(mktemp -d)
echo "Testing in $TEST_DIR"

cp -r . $TEST_DIR/repo
cd $TEST_DIR/repo

# Build first
cargo build

# Init git
git init
git config user.email "test@example.com"
git config user.name "Test User"
git add .
git commit -m "Initial commit"

# Init beads
./target/debug/bl init

# Create issue
./target/debug/bl create "Sync Test Task"

# Sync
./target/debug/bl sync

# Verify files exist
if [ -f "issues.jsonl" ]; then
    echo "issues.jsonl created"
else
    echo "issues.jsonl MISSING"
    exit 1
fi

if [ -f "sync_base.jsonl" ]; then
    echo "sync_base.jsonl created"
else
    echo "sync_base.jsonl MISSING"
    exit 1
fi

# Verify content
grep "Sync Test Task" issues.jsonl
grep "Sync Test Task" sync_base.jsonl

# Verify git commit
git log --oneline | grep "bl sync"

echo "Sync Verified Successfully"
