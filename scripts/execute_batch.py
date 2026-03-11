"""
Execute multiple Jupyter code cells using nbclient.
This preserves kernel state across cells (variables defined in cell 0 are available in cell 1).
"""

import sys
import json
import argparse
import nbformat
from nbclient import NotebookClient


def execute_batch(cells_code, kernel_name='python3', timeout=30):
    """
    Execute multiple code snippets in sequence using a single kernel.

    Args:
        cells_code: List of code strings to execute
        kernel_name: Kernel to use (default: python3)
        timeout: Timeout in seconds per cell

    Returns:
        list of dicts with execution results for each cell
    """
    # Create notebook with multiple cells
    nb = nbformat.v4.new_notebook()
    nb.cells = [nbformat.v4.new_code_cell(code) for code in cells_code]

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
        allow_errors=True,  # Continue on errors
    )

    try:
        # Execute all cells (kernel state preserved across cells)
        client.execute()

        # Extract results from each cell
        results = []
        for cell in nb.cells:
            outputs = cell.get('outputs', [])
            execution_count = cell.get('execution_count')

            # Check for errors
            error_output = next(
                (o for o in outputs if o.get('output_type') == 'error'),
                None
            )

            if error_output:
                results.append({
                    'success': False,
                    'outputs': outputs,
                    'execution_count': execution_count,
                    'error': {
                        'ename': error_output.get('ename', 'Error'),
                        'evalue': error_output.get('evalue', ''),
                        'traceback': error_output.get('traceback', [])
                    }
                })
            else:
                results.append({
                    'success': True,
                    'outputs': outputs,
                    'execution_count': execution_count,
                    'error': None
                })

        return results

    except Exception as e:
        # Return error for all cells
        return [{
            'success': False,
            'outputs': [],
            'execution_count': None,
            'error': {
                'ename': type(e).__name__,
                'evalue': str(e),
                'traceback': []
            }
        } for _ in cells_code]


def main():
    parser = argparse.ArgumentParser(
        description='Execute multiple Python code cells using Jupyter kernel'
    )
    parser.add_argument(
        'cells',
        nargs='*',  # Changed from '+' to '*' to allow --from-json without positional args
        help='Code for each cell (or use --from-json)'
    )
    parser.add_argument(
        '--from-json',
        action='store_true',
        help='Read cells from stdin as JSON array'
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
        help='Timeout in seconds per cell (default: 30)'
    )

    args = parser.parse_args()

    # Get cell codes
    if args.from_json:
        cells_code = json.load(sys.stdin)
    elif args.cells:
        cells_code = args.cells
    else:
        parser.error("Must provide cells as arguments or use --from-json")

    # Execute batch
    results = execute_batch(cells_code, kernel_name=args.kernel, timeout=args.timeout)

    # Output as JSON
    print(json.dumps(results, indent=2))

    # Exit with error code if any cell failed
    any_failed = any(not r['success'] for r in results)
    sys.exit(1 if any_failed else 0)


if __name__ == '__main__':
    main()
