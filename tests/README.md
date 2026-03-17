# Integration Tests

This directory contains integration tests for the nb-cli tool's local mode operations.

## Test Structure

```
tests/
├── integration_local_mode.rs   # Core CLI operations (no execution)
├── integration_execution.rs    # Execution tests (requires Python)
├── test_helpers.rs             # Test utilities and venv setup
├── setup_test_env.sh           # Script to setup test environment
├── fixtures/                   # Test notebook fixtures
│   ├── empty.ipynb             # Empty notebook with no cells
│   ├── basic.ipynb             # Basic notebook with one empty cell
│   ├── with_code.ipynb         # Notebook with code cells
│   ├── mixed_cells.ipynb       # Notebook with mixed cell types
│   ├── with_outputs.ipynb      # Notebook with cell outputs
│   ├── for_execution.ipynb     # Simple execution test notebook
│   └── with_error.ipynb        # Notebook with error cell
├── .test-venv/                 # Test virtual environment (auto-created)
└── README.md                   # This file
```

## Running Tests

### Setup (One Time)

For execution tests, you need to setup the Python environment:

```bash
# Option 1: Use the setup script (requires uv)
./tests/setup_test_env.sh

# Option 2: Manual setup
uv venv tests/.test-venv
tests/.test-venv/bin/pip install ipykernel
```

**Note**: If you don't have `uv` installed:
```bash
curl -LsSf https://astral.sh/uv/install.sh | sh
```

### Run All Tests

Run all integration tests (both local and execution):
```bash
cargo test
```

### Run Specific Test Suites

Run only local mode tests (no Python required):
```bash
cargo test --test integration_local_mode
```

Run only execution tests (requires Python venv):
```bash
cargo test --test integration_execution
```

### Run Specific Tests

```bash
cargo test --test integration_local_mode test_create_empty_notebook
cargo test --test integration_execution test_execute_single_cell
```

### Run with Output

```bash
cargo test --test integration_local_mode -- --nocapture
cargo test --test integration_execution -- --nocapture
```

### Execution Tests Behavior

Execution tests will automatically:
- ✅ **Skip gracefully** if Python environment is not set up
- ✅ **Auto-create venv** with `uv` if available
- ✅ **Install dependencies** (ipykernel for Python kernel)
- ✅ **Use isolated environment** (tests/.test-venv)

If execution tests are skipped, you'll see:
```
⚠️  Skipping test: execution environment not available
```
