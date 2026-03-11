"""
Execute Jupyter code using nbclient.
This script is called by the Rust CLI to execute notebook cells locally.
"""

import sys
import json
import argparse
import nbformat
from nbclient import NotebookClient
from nbclient.exceptions import CellExecutionError


def execute_code(code, kernel_name='python3', timeout=30):
    """
    Execute a single code snippet using nbclient.

    Args:
        code: Python code to execute
        kernel_name: Kernel to use (default: python3)
        timeout: Timeout in seconds

    Returns:
        dict with execution results in the format:
        {
            'success': bool,
            'outputs': [...],
            'execution_count': int or None,
            'error': {...} or None
        }
    """
    # Create minimal notebook with one cell
    nb = nbformat.v4.new_notebook()
    cell = nbformat.v4.new_code_cell(code)
    nb.cells = [cell]

    # Set kernel metadata
    nb.metadata['kernelspec'] = {
        'name': kernel_name,
        'display_name': kernel_name,
        'language': 'python'
    }

    # Create notebook client
    client = NotebookClient(
        nb,
        timeout=timeout,
        kernel_name=kernel_name,
        allow_errors=True,  # Don't raise on errors, collect them
    )

    try:
        # Execute the notebook (starts kernel, executes all cells, shuts down)
        # This is the recommended way to use nbclient - it handles the full lifecycle
        client.execute()

        # Extract results from the executed cell
        outputs = cell.get('outputs', [])
        execution_count = cell.get('execution_count')

        # Check if there was an error in the outputs
        error_output = next(
            (o for o in outputs if o.get('output_type') == 'error'),
            None
        )

        if error_output:
            # Execution had an error
            result = {
                'success': False,
                'outputs': outputs,
                'execution_count': execution_count,
                'error': {
                    'ename': error_output.get('ename', 'Error'),
                    'evalue': error_output.get('evalue', ''),
                    'traceback': error_output.get('traceback', [])
                }
            }
        else:
            # Execution succeeded
            result = {
                'success': True,
                'outputs': outputs,
                'execution_count': execution_count,
                'error': None
            }

    except CellExecutionError as e:
        # This shouldn't happen since allow_errors=True, but handle it anyway
        outputs = cell.get('outputs', [])
        execution_count = cell.get('execution_count')

        error_output = next(
            (o for o in reversed(outputs) if o.get('output_type') == 'error'),
            None
        )

        if error_output:
            error_info = {
                'ename': error_output.get('ename', 'Error'),
                'evalue': error_output.get('evalue', str(e)),
                'traceback': error_output.get('traceback', [])
            }
        else:
            error_info = {
                'ename': 'CellExecutionError',
                'evalue': str(e),
                'traceback': []
            }

        result = {
            'success': False,
            'outputs': outputs,
            'execution_count': execution_count,
            'error': error_info
        }

    except Exception as e:
        # Kernel startup or other error
        result = {
            'success': False,
            'outputs': [],
            'execution_count': None,
            'error': {
                'ename': type(e).__name__,
                'evalue': str(e),
                'traceback': []
            }
        }

    return result


def main():
    parser = argparse.ArgumentParser(
        description='Execute Python code using Jupyter kernel'
    )
    parser.add_argument(
        'code',
        help='Python code to execute (or "-" to read from stdin)'
    )
    parser.add_argument(
        '--kernel',
        default='python3',
        help='Kernel name (default: python3)'
    )
    parser.add_argument(
        '--timeout',
        type=int,
        default=30,
        help='Timeout in seconds (default: 30)'
    )

    args = parser.parse_args()

    # Read code from stdin if "-" is passed
    code = args.code
    if code == '-':
        code = sys.stdin.read()

    # Execute code
    result = execute_code(code, kernel_name=args.kernel, timeout=args.timeout)

    # Output as JSON
    print(json.dumps(result, indent=2))

    # Exit with error code if execution failed
    sys.exit(0 if result['success'] else 1)


if __name__ == '__main__':
    main()
