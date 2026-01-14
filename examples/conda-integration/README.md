# Conda Integration with rv

This example demonstrates how to use rv with conda/mamba/micromamba environments.

## Quick Start

### 1. Using an Existing Environment

If you already have a conda environment with R:

```bash
# Activate your conda environment
conda activate my-r-env

# Create a new rv project
cd /path/to/your/project
rv init

# Edit rproject.toml to add conda_env
echo 'conda_env = "my-r-env"' >> rproject.toml

# Add dependencies
rv add dplyr ggplot2

# Sync packages (installs to the conda environment's library)
rv sync
```

### 2. Creating a New Environment with rv

If you don't have a conda environment yet:

```bash
# Create a new rv project
cd /path/to/your/project
rv init

# Edit rproject.toml to add conda_env
echo 'conda_env = "my-project"' >> rproject.toml

# Add dependencies
rv add dplyr ggplot2

# Sync with auto-create (this will create the conda environment if it doesn't exist)
rv sync --condaenv my-project --auto-create
```

## Configuration

Edit your `rproject.toml` to specify the conda environment:

```toml
[project]
name = "my-project"
r_version = "4.4.1"
conda_env = "my-project"  # The conda environment to use

[[repositories]]
alias = "posit"
url = "https://packagemanager.posit.co/cran/2024-12-16/"

dependencies = [
    "dplyr",
    "ggplot2",
    "data.table"
]
```

## Usage

### Basic Commands

```bash
# Sync dependencies (installs to conda environment)
rv sync

# Add a new package
rv add dplyr

# Add a package with version constraint
rv add "ggplot2>=3.5.0"

# Add from a specific repository
rv add package --repository bioc

# Plan (dry-run) what would be installed
rv plan
```

### Conda-Specific Commands

```bash
# Use a specific conda environment
rv sync --condaenv my-env

# Auto-create environment if it doesn't exist
rv sync --condaenv my-env --auto-create
```

## How It Works

### Two-Level Installation

1. **Conda Environment Creation** (One-time):
   ```bash
   # rv creates the conda environment with R
   micromamba create -n my-project -c conda-forge -c r r-base=4.4.1
   ```

2. **R Package Installation** (Every sync):
   ```bash
   # rv installs R packages to the conda environment's library
   rv sync  # Downloads and installs dplyr, ggplot2, etc.
   ```

### What Gets Installed Where

- **R Runtime**: Installed by conda in the environment
  - Location: `$CONDA_PREFIX/bin/R`
  - Version: Specified in `rproject.toml`

- **R Packages**: Installed by rv
  - Location: `$CONDA_PREFIX/lib/R/library`
  - Source: CRAN, R-Universe, Bioconductor, etc.
  - Managed by: rv

## Example Workflow

### Bioinformatics Project

```bash
# 1. Create project
mkdir rnaseq-analysis && cd rnaseq-analysis
rv init

# 2. Configure
cat > rproject.toml << 'EOF'
[project]
name = "rnaseq-analysis"
r_version = "4.4.1"
conda_env = "rnaseq-analysis"

[[repositories]]
alias = "posit"
url = "https://packagemanager.posit.co/cran/2024-12-16/"

[[repositories]]
alias = "bioc"
url = "https://packagemanager.posit.co/bioc/2024-12-16/"

dependencies = [
    "dplyr",
    "tidyr",
    "ggplot2",
    "DESeq2",
    "tximport"
]
EOF

# 3. Create environment and install packages
rv sync --condaenv rnaseq-analysis --auto-create

# 4. Run your analysis
Rscript run_analysis.R
```

### Script.R

```r
#!/usr/bin/env Rscript
# This script runs in the conda environment automatically

library(DESeq2)
library(dplyr)
library(ggplot2)

# Your analysis code here...
cat("Analysis complete!\n")
```

## Best Practices

### 1. Environment Naming
- Use the project name as the environment name for clarity
- Example: Project `my-analysis` → Environment `my-analysis`

### 2. Version Pinning
- Pin R version in `rproject.toml` for reproducibility
- Example: `r_version = "4.4.1"`

### 3. Repository Configuration
- Add only the repositories you need
- Order matters: first repository has highest priority

### 4. No Mixed Package Management
- Use either conda OR rv for R packages, not both in the same environment
- This prevents conflicts and version mismatches

## Troubleshooting

### Error: "Conda environment not found"

Solution:
```bash
# Either create the environment first
conda create -n my-env r-base=4.4.1

# Or use auto-create
rv sync --condaenv my-env --auto-create
```

### Error: "R version not compatible"

Solution:
- Ensure `r_version` in `rproject.toml` matches the conda environment's R version
- Check: `conda run -n my-env R --version`

### Error: "Permission denied" when writing to conda environment

Solution:
- Use a user-level conda environment instead of system-level
- Create with: `conda create -p ~/conda_envs/my-env r-base=4.4.1`

## Comparison with renv

| Feature | rv + conda | renv |
|---------|-------------|------|
| Environment isolation | ✓ | ✓ |
| R version management | ✓ (via conda) | ✓ |
| Binary packages | ✓ | ✓ |
| R package manager | ✓ | ✓ |
| Cross-platform | ✓ | ✓ |
| Speed | Faster (conda binaries) | Slower |
| Lock file | ✓ (rv.lock) | ✓ (renv.lock) |
| Automatic dependency discovery | ✗ | ✓ |
| Conda integration | ✓ | ✗ |

## Advanced Usage

### Using with CI/CD

```yaml
# .github/workflows/r.yml
- name: Setup R environment
  run: |
    rv sync --condaenv my-project --auto-create

- name: Run tests
  run: |
    conda run -n my-project Rscript test.R
```

### Using with Docker

```dockerfile
FROM continuumio/miniconda3

# Install rv
COPY --from=ghcr.io/r-lib/rv:latest /usr/local/bin/rv /usr/local/bin/

# Setup project
COPY . /app
WORKDIR /app

# Create environment and install packages
RUN rv sync --condaenv my-project --auto-create

# Run
CMD ["conda", "run", "-n", "my-project", "Rscript", "script.R"]
```

## Tips

1. **Speed**: rv prioritizes binary packages from repositories, which is much faster than compiling from source
2. **Reproducibility**: The `rv.lock` file ensures consistent package versions across environments
3. **Flexibility**: You can use any conda environment, not just those created by rv
4. **Isolation**: Each project can have its own conda environment
