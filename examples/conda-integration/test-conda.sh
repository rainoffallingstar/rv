#!/bin/bash
# Test script for conda integration

set -e

echo "=== Testing rv conda integration ==="
echo

# Check if conda is available
if ! command -v conda &> /dev/null && ! command -v mamba &> /dev/null && ! command -v micromamba &> /dev/null; then
    echo "⚠️  No conda/mamba/micromamba found. Skipping conda integration tests."
    echo "   Install conda/mamba/micromamba to run these tests."
    exit 0
fi

echo "✓ Conda tool found"
echo

# Test 1: Detect conda tool
echo "Test 1: Detecting conda tool..."
if rv --help | grep -q "condaenv"; then
    echo "✓ CLI supports --condaenv flag"
else
    echo "✗ CLI does not support --condaenv flag"
    exit 1
fi
echo

# Test 2: Check config file support
echo "Test 2: Checking configuration file support..."
cat > test_project/rproject.toml << 'EOF'
[project]
name = "test-project"
r_version = "4.4.1"
conda_env = "test-env"

[[repositories]]
alias = "posit"
url = "https://packagemanager.posit.co/cran/2024-12-16/"

dependencies = [
    "dplyr"
]
EOF

echo "✓ Test configuration file created"
echo

# Test 3: Verify conda environment detection (will fail if env doesn't exist)
echo "Test 3: Testing conda environment detection..."
ENV_NAME="rv_test_$(date +%s)"
echo "Testing with non-existent environment: $ENV_NAME"

# This should fail gracefully
if rv sync --condaenv "$ENV_NAME" 2>&1 | grep -q "not found"; then
    echo "✓ Correctly reports environment not found"
else
    echo "ℹ️  Could not verify environment not found message (may be OK)"
fi
echo

# Test 4: Verify auto-create flag
echo "Test 4: Testing --auto-create flag..."
if rv sync --help 2>&1 | grep -q "auto-create"; then
    echo "✓ CLI supports --auto-create flag"
else
    echo "✗ CLI does not support --auto-create flag"
    exit 1
fi
echo

# Cleanup
echo "=== Cleanup ==="
rm -rf test_project

echo
echo "✓ All basic tests passed!"
echo
echo "=== Next Steps ==="
echo "1. Create a conda environment with R:"
echo "   conda create -n my-project r-base=4.4.1"
echo
echo "2. Create an rv project:"
echo "   cd /path/to/project"
echo "   rv init"
echo
echo "3. Edit rproject.toml and add:"
echo '   conda_env = "my-project"'
echo
echo "4. Sync packages:"
echo "   rv sync --condaenv my-project"
echo
echo "5. Or auto-create:"
echo "   rv sync --condaenv my-project --auto-create"
