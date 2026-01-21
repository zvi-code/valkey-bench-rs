# Installation Guide

Quick setup guide for the Valkey Benchmark (valkey-bench-rs) environment.

## Quick Start

### Step 1: Install System Dependencies

```bash
sudo apt-get update
sudo apt-get install -y python3 python3-pip python3-venv python3-numpy python3-yaml libhdf5-dev pkg-config libssl-dev
```

### Step 2: Install Rust

```bash
# Install Rust via rustup (recommended)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# Verify installation
rustc --version  # Should show 1.70+
```

### Step 3: Clone Repository

```bash
git clone https://github.com/zvi-code/valkey-bench-rs.git
cd valkey-bench-rs
```

### Step 4: Configure Storage (Important for Large Datasets)

Vector datasets can be very large (10GB-40GB+). **Do not use your boot volume** for storing datasets.

**For small datasets (< 5GB):** The default `./datasets/` directory is fine.

**For medium/large datasets:** Mount a dedicated storage volume:

```bash
# Example: Mount an additional EBS or NVMe volume
sudo mkdir -p /mnt/data
sudo mount /dev/nvme1n1 /mnt/data  # Or your volume device
sudo chown -R $USER:$USER /mnt/data

# Configure dataset paths
export DATASET_PATH=/mnt/data/datasets
export BUILD_DATASET_PATH=/mnt/data/build-datasets
mkdir -p $DATASET_PATH $BUILD_DATASET_PATH
```

> **Tip**: Add the exports to `~/.bashrc` for persistence. See [Appendix B](#appendix-b-nvme-storage-setup-large-datasets) for detailed NVMe setup.

### Step 5: Build valkey-bench-rs

```bash
# Release build (optimized)
cargo build --release

# Run tests
cargo test
```

### Step 6: Verify Build

```bash
./target/release/valkey-bench-rs --help
```

### Step 7: Setup Python Environment (for Dataset Preparation)

```bash
# Run the prereq script to set up VectorDBBench and dependencies
./prep_datasets/prereq-vectordbbench.sh
```

This script:
- Creates a Python virtual environment (`venv/`)
- Installs VectorDBBench with MemoryDB support
- Installs dataset conversion dependencies (h5py, pandas, pyarrow, numpy)

### Step 8: Download a Dataset

```bash
./prep_datasets/dataset.sh get mnist
```

### Step 9: Create Test Datasets

```bash
# Create test datasets (YAML schema + binary data)
python3 prep_datasets/create_test_dataset.py -o datasets
```

This creates three test datasets:
- `test_small.yaml` + `test_small.bin` - 100 vectors, 32 dimensions
- `test_mnist.yaml` + `test_mnist.bin` - 1000 vectors, 784 dimensions
- `test_hash.yaml` + `test_hash.bin` - 100 records with vector + category + price fields

### Step 10: Run a Test Benchmark

```bash
# Test connectivity (replace with your server address)
./target/release/valkey-bench-rs -h $HOST -p 6379 -t ping -n 1000 -q

# Run vector benchmark with schema-driven dataset
./target/release/valkey-bench-rs -h $HOST -p 6379 \
  --schema datasets/test_small.yaml \
  --data datasets/test_small.bin \
  --search-index test-idx \
  --search-prefix test: \
  -t vec-load -n 100

# Run vector query benchmark
./target/release/valkey-bench-rs -h $HOST -p 6379 \
  --schema datasets/test_small.yaml \
  --data datasets/test_small.bin \
  --search-index test-idx \
  --search-prefix test: \
  -t vec-query -n 100 -k 5
```

---

## Next Steps

- **Dataset Management**: See [DATASETS.md](DATASETS.md) for downloading and preparing datasets
- **Running Benchmarks**: See [BENCHMARKING.md](BENCHMARKING.md) for benchmark usage
- **Advanced Features**: See [ADVANCED.md](ADVANCED.md) for optimizer and metadata filtering
- **Examples**: See [EXAMPLES.md](EXAMPLES.md) for comprehensive feature examples

---

## Appendix A: System Requirements

### Minimum Requirements

- **OS**: Ubuntu 20.04+ or compatible Linux
- **CPU**: x86_64 or ARM64 (aarch64)
- **RAM**: 8GB minimum, 16GB+ recommended
- **Storage**: 50GB+ for datasets
- **Rust**: 1.70 or newer

### Recommended for Production Benchmarks

- **RAM**: 32GB+ for large datasets
- **Storage**: 200GB+ SSD/NVMe
- **Network**: Good bandwidth for downloading datasets (up to 40GB+)

### ARM64 Support

This benchmark is optimized for ARM64 servers, particularly AWS Graviton:

| Use Case | Recommended Instance |
|----------|---------------------|
| Development & Testing | c7g.2xlarge (8 vCPU, 16GB) |
| Production Benchmarks | c7g.8xlarge (32 vCPU, 64GB) |
| Large Datasets | c7gd.8xlarge (+ 950GB NVMe) |
| Memory-Intensive | r7g.8xlarge (32 vCPU, 256GB) |

---

## Appendix B: NVMe Storage Setup (Large Datasets)

For datasets larger than 10GB, using NVMe instance storage is recommended on AWS.

### Format and Mount NVMe Drive

```bash
# Identify the NVMe drive
lsblk

# Create mount point and format
sudo mkdir -p /mnt/data
sudo mkfs.ext4 /dev/nvme1n1
sudo mount -o defaults,noatime,discard /dev/nvme1n1 /mnt/data
sudo chown -R $USER:$USER /mnt/data

# Create dataset directories
mkdir -p /mnt/data/datasets /mnt/data/build-datasets
```

### Configure Dataset Paths

Set environment variables to use NVMe storage:

```bash
export DATASET_PATH=/mnt/data/datasets
export BUILD_DATASET_PATH=/mnt/data/build-datasets
```

Add to `~/.bashrc` for persistence.

> **Note**: If `/mnt/data` is not available, the dataset manager automatically uses local project directories (`datasets/raw/` and `datasets/`).

---

## Appendix C: Build Options

### Debug Build

```bash
cargo build
```

### Release Build with Optimizations

```bash
cargo build --release
```

### Build with rustls TLS Backend

```bash
# Use rustls instead of native-tls (for environments without OpenSSL)
cargo build --release --features rustls-backend
```

### Check for Compilation Issues

```bash
cargo check
cargo clippy
```

---

## Appendix D: Troubleshooting

### Build Issues

**Rust not found**
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

**OpenSSL/TLS issues**
```bash
sudo apt-get install libssl-dev pkg-config
# Or use rustls backend:
cargo build --release --features rustls-backend
```

**HDF5 not found (for dataset preparation)**
```bash
sudo apt-get install libhdf5-dev libhdf5-serial-dev
```

**Compilation errors**
```bash
# Update Rust toolchain
rustup update

# Clean and rebuild
cargo clean && cargo build --release
```

### Runtime Issues

**Connection refused**
```bash
# Test basic connectivity
./target/release/valkey-bench-rs --cli -h $HOST PING
```

**TLS certificate errors**
```bash
# Skip verification for testing
./target/release/valkey-bench-rs --tls --tls-skip-verify -h $HOST PING
```

### Python Issues

**ModuleNotFoundError: No module named 'vectordb_bench'**
```bash
# Run the prereq script to set up the environment
./prep_datasets/prereq-vectordbbench.sh

# Or manually:
source venv/bin/activate
pip install vectordb-bench[memorydb]
```

**ImportError: libhdf5.so not found**
```bash
sudo apt-get install libhdf5-dev libhdf5-serial-dev
pip uninstall h5py && pip install h5py --no-binary h5py
```

**PyArrow installation fails**
```bash
pip install pyarrow==14.0.0
```

### Storage Issues

**No space left on device**
- Use NVMe storage (see Appendix B)
- Clean build artifacts: `cargo clean`
- Remove old datasets: `rm datasets/*.bin`

---

## Appendix E: Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `DATASET_PATH` | Raw downloads and HDF5 cache | `./datasets/raw/` |
| `BUILD_DATASET_PATH` | Final binary datasets | `./datasets/` |
| `VENV_PATH` | Python virtual environment | `./venv/` |
| `RUST_LOG` | Rust logging level (debug, info, warn, error) | `warn` |

---

## Appendix F: Clean Uninstall

```bash
# Remove build artifacts
cargo clean

# Remove Python environment
rm -rf venv

# Remove datasets
rm -rf datasets

# If using NVMe storage
rm -rf /mnt/data/datasets /mnt/data/build-datasets
sudo umount /mnt/data
```
