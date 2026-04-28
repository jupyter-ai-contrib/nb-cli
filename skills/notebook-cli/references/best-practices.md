# Best Practices for Authoring Jupyter Notebooks

This document synthesizes research-based best practices for creating high-quality, reproducible Jupyter notebooks. These guidelines are derived from comprehensive studies of over 1 million notebooks and focus on standalone notebook development.

## Table of Contents
1. [Narrative and Documentation](#narrative-and-documentation)
2. [Code Organization](#code-organization)
3. [Code Quality and Style](#code-quality-and-style)
4. [Dependency Management](#dependency-management)
5. [Execution and Reproducibility](#execution-and-reproducibility)
6. [Naming Conventions](#naming-conventions)

---

## 1. Narrative and Documentation

### Tell a Story for an Audience
Create a computational narrative with clear beginning, middle, and end:
- **Beginning**: Introduce the goal, topic, research question, or problem
- **Middle**: Describe analytical steps, methodologies, and **why** you chose them
- **End**: Interpret results, draw conclusions, suggest next steps
- Use complete sentences and paragraphs, rather than bullet points.
- Keep it concise, but give enough detail that the reader can follow the story
- Explain reasoning, not just actions
- Write for your future self—err on the side of over-explanation

### Document in Real-Time
- Document explorations as you go, not after completion
- Record why you chose specific parameter values
- Explain what made intermediate results interesting

### Use Markdown Effectively
- Place markdown cells **before** code cells to explain what follows
- Use descriptive headers to organize notebooks into sections
- Create a table of contents for longer notebooks (>100 lines)
- Include equations, links, and figures where appropriate
- Ensure markdown accurately describes the associated code

---

## 2. Code Organization

### Structure Cells Clearly
- Make each cell perform one logical task (load data, create plot, fit model)
- Think of each cell as one paragraph or function
- Limit cells to ~100 lines maximum
- Use code comments for low-level documentation within cells
- Avoid many short, fragmented one-line cells

### Avoid Code Duplication
- **Wrap reused code in functions and classes**
- Define utility functions and classes early in the notebook
- Benefits: easier maintenance, improved readability, better debugging

### Organize Cells Logically
- Place imports at the top
- Place key variables and configuration near the beginning
- Follow top-to-bottom flow: data loading → processing → analysis → visualization → conclusions
- Avoid forward references (cells depending on later cells)
- Keep related code together in adjacent cells

**Research Finding**: Out-of-order execution is a major source of reproducibility issues.

---

## 3. Code Quality and Style

### Follow PEP 8 Style Conventions for Python code
**Critical Rules**:
- Limit line length to 79-100 characters
- Use proper whitespace after commas, semicolons, colons
- Use 4 spaces for indentation
- Avoid whitespace after `(` or before `)`
- Add blank lines between function/class definitions
- Use whitespace around operators
- Start block comments with `# `
- Place inline comments with at least 2 spaces before them

**Research Finding**: Widespread PEP 8 violations found even in curated notebooks.

### Maintain Clean Code
- Remove **unused variables** (defined but never referenced)
- Replace hard-coded values with named constants towards the top of the notebook
- Add error handling for external operations (file I/O, network calls)
- Use descriptive variable and function names

### Avoid Hidden State

Hidden state occurs when execution state doesn't match visible code:
1. Re-executing a cell multiple times (counter skips)
2. Editing a cell after execution without re-running
3. Deleting a cell after execution

**Prevention**:
- Restart kernel and run all cells regularly during development
- Before finishing: **Restart Kernel → Run All Cells** → Verify outputs
- Avoid manual interventions after execution
- Use execution counter continuity as a quality check
---

## 4. Dependency Management

### Install Dependencies Inside the Notebook

**NEVER** install packages by running `pip install` or `python -m pip install` in the shell. This makes the notebook non-reproducible for other users. Instead, add a code cell near the top of the notebook using `%pip install` (the IPython magic), which always targets the correct kernel environment:

```bash
nb cell add notebook.ipynb -s '@@markdown
## Setup
@@code
%pip install matplotlib numpy pandas' --insert-at 0
```

Then execute the setup cell before running the rest of the notebook:

```bash
nb execute notebook.ipynb --cell-index 1
nb execute notebook.ipynb
```

### Add Cells in Bulk, Not One at a Time

Use multi-cell sentinels (`@@code`, `@@markdown`, `@@raw`) to add all related cells in a single `nb cell add` call. Do NOT add cells one by one in separate commands — it is slower and produces unnecessary tool calls. Plan the full section of cells, then add them together:

```bash
nb cell add notebook.ipynb -s '@@markdown
# Matplotlib Tutorial

This notebook demonstrates core plotting concepts with matplotlib.
@@code
%pip install matplotlib numpy
@@markdown
## Basic Line Plot

Create a simple sine wave to demonstrate `plt.plot()`.
@@code
import matplotlib.pyplot as plt
import numpy as np

x = np.linspace(0, 10, 100)
plt.plot(x, np.sin(x))
plt.title("Sine Wave")
plt.show()'
```

---

## 5. Execution and Reproducibility

### Execute Cells in Order
Design notebooks to execute sequentially from top to bottom:
- Avoid dependencies on out-of-order execution
- Use **Restart & Run All** frequently during development
- Ensure execution counters are sequential (no gaps, no repeats)
- If you explore out of order, re-run entire notebook before saving

**Warning Signs**:
- Execution counters out of numerical order
- Cells with `[*]` (still executing)
- Gaps in execution counter sequence
- Repeated execution counter values

**Research Finding**: Only 24% of notebooks executed without errors; only 4% produced identical results upon re-execution.

### Design for Re-execution
- Place variable declarations at the top (especially those that change between runs)
- Perform data preparation within the notebook
- Avoid manual interventions (manual downloads, GUI interactions)
- Document expected inputs and outputs
- Provide examples of parameter values

### Ensure Complete Execution
- Test regularly: **Restart Kernel → Run All Cells**
- Fix execution errors before sharing
- Handle external dependencies gracefully (network failures, missing files)
- Document cells that cannot be automated
- All execution counters should be sequential and complete
- All outputs should be present and correct

---

## 6. Naming Conventions

### Use Meaningful Notebook Names
Give notebooks descriptive, professional names:
- **Avoid**: "Untitled", "Untitled1", "Copy of X"
- **Use**: Descriptive names like `customer_segmentation_analysis.ipynb`
- **Consider**: Numeric prefixes for sequential analysis: `01_data_loading.ipynb`
- **Avoid**: Special characters: `?`, `*`, `<`, `>`, `|`
- **Recommended**: A-Z, a-z, 0-9, `_`, `-`, `.`
- **Case**: Use snake_case or kebab-case

**Examples**:
- Good: `exploratory_data_analysis.ipynb`, `01-data-preprocessing.ipynb`
- Bad: `Untitled.ipynb`, `analysis copy.ipynb`, `test?.ipynb`