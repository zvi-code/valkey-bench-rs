# Third-Party Licenses and Dataset Attributions

This document lists all third-party dependencies, tools, and datasets used by valkey-bench-rs, along with their licenses and terms of use.

---

## Rust Dependencies

The following Rust crates are used by valkey-bench-rs:

### clap
- **Source**: https://github.com/clap-rs/clap
- **License**: MIT OR Apache-2.0
- **Usage**: Command-line argument parsing

### hdrhistogram
- **Source**: https://github.com/HdrHistogram/HdrHistogram_rust
- **License**: MIT OR Apache-2.0
- **Usage**: High Dynamic Range histogram for latency metrics

### memmap2
- **Source**: https://github.com/RazrFalcon/memmap2-rs
- **License**: MIT OR Apache-2.0
- **Usage**: Memory-mapped file access for datasets

### mio
- **Source**: https://github.com/tokio-rs/mio
- **License**: MIT
- **Usage**: Event-driven I/O for non-blocking network operations

### parking_lot
- **Source**: https://github.com/Amanieu/parking_lot
- **License**: MIT OR Apache-2.0
- **Usage**: Efficient synchronization primitives

### crossbeam-channel
- **Source**: https://github.com/crossbeam-rs/crossbeam
- **License**: MIT OR Apache-2.0
- **Usage**: Multi-producer multi-consumer channels

### fastrand
- **Source**: https://github.com/smol-rs/fastrand
- **License**: MIT OR Apache-2.0
- **Usage**: Fast random number generation

### indicatif
- **Source**: https://github.com/console-rs/indicatif
- **License**: MIT
- **Usage**: Progress bar and spinner display

### tracing
- **Source**: https://github.com/tokio-rs/tracing
- **License**: MIT
- **Usage**: Application-level tracing and logging

### serde / serde_json
- **Source**: https://github.com/serde-rs/serde
- **License**: MIT OR Apache-2.0
- **Usage**: Serialization and JSON output

### native-tls
- **Source**: https://github.com/sfackler/rust-native-tls
- **License**: MIT OR Apache-2.0
- **Usage**: TLS/SSL support (default backend)

### rustls
- **Source**: https://github.com/rustls/rustls
- **License**: MIT OR Apache-2.0
- **Usage**: TLS/SSL support (optional alternative backend)

### thiserror / anyhow
- **Source**: https://github.com/dtolnay/thiserror
- **License**: MIT OR Apache-2.0
- **Usage**: Error handling

### tabled
- **Source**: https://github.com/zhiburt/tabled
- **License**: MIT
- **Usage**: Table formatting for console output

---

## Python Dependencies (Dataset Preparation)

### VectorDBBench
- **Source**: https://github.com/zilliztech/VectorDBBench
- **License**: MIT License
- **Copyright**: Zilliz
- **Usage**: Downloading modern embedding datasets (Cohere, OpenAI embeddings)

### ANN-Benchmarks
- **Source**: https://github.com/erikbern/ann-benchmarks
- **Website**: http://ann-benchmarks.com/
- **License**: MIT License
- **Authors**: Martin Aumueller, Erik Bernhardsson, Alexander Faithfull
- **Usage**: Pre-generated HDF5 datasets
- **Citation**: M. Aumueller, E. Bernhardsson, A. Faithfull: "ANN-Benchmarks: A Benchmarking Tool for Approximate Nearest Neighbor Algorithms." Information Systems 2019.

### h5py
- **Source**: https://github.com/h5py/h5py
- **License**: BSD-3-Clause
- **Usage**: HDF5 file handling

### numpy
- **Source**: https://numpy.org/
- **License**: BSD-3-Clause
- **Usage**: Numerical computations for dataset conversion

### pandas / pyarrow
- **Source**: https://pandas.pydata.org/, https://arrow.apache.org/
- **License**: BSD-3-Clause / Apache-2.0
- **Usage**: Parquet file handling

---

## Datasets

All datasets are used for **research and benchmarking purposes only**. The datasets are publicly available and free to use for non-commercial research.

### 1. SIFT (Scale-Invariant Feature Transform)
- **Source**: http://corpus-texmex.irisa.fr/
- **Dimensions**: 128
- **Vectors**: 1M base vectors, 10K queries
- **License**: Public domain for research use
- **Original Authors**: Herve Jegou, Laurent Amsaleg
- **Description**: SIFT features extracted from images
- **Citation**: H. Jegou, M. Douze, C. Schmid. "Product quantization for nearest neighbor search." IEEE TPAMI, 2011

### 2. GIST
- **Source**: http://corpus-texmex.irisa.fr/
- **Dimensions**: 960
- **Vectors**: 1M base vectors, 1K queries
- **License**: Public domain for research use
- **Description**: GIST global image descriptors

### 3. GloVe (Global Vectors for Word Representation)
- **Source**: https://nlp.stanford.edu/projects/glove/
- **Dimensions**: 25, 50, 100, 200
- **Vectors**: ~1.2M words
- **License**: Public Domain Dedication and License (PDDL)
- **Authors**: Jeffrey Pennington, Richard Socher, Christopher D. Manning (Stanford NLP)
- **Citation**: Jeffrey Pennington, Richard Socher, and Christopher D. Manning. 2014. "GloVe: Global Vectors for Word Representation."

### 4. Deep1B / Deep-Image-96
- **Source**: http://sites.skoltech.ru/compvision/noimi/
- **Dimensions**: 96
- **License**: Public domain for research use
- **Description**: Deep learning features from images

### 5. Fashion-MNIST
- **Source**: https://github.com/zalandoresearch/fashion-mnist
- **Dimensions**: 784 (28x28 images)
- **Vectors**: 60K training, 10K test
- **License**: MIT License
- **Authors**: Zalando Research
- **Citation**: Han Xiao, Kashif Rasul, Roland Vollgraf. "Fashion-MNIST: a Novel Image Dataset for Benchmarking Machine Learning Algorithms." arXiv:1708.07747

### 6. MNIST
- **Source**: http://yann.lecun.com/exdb/mnist/
- **Dimensions**: 784 (28x28 images)
- **Vectors**: 60K training, 10K test
- **License**: Creative Commons Attribution-Share Alike 3.0
- **Authors**: Yann LeCun, Corinna Cortes, Christopher J.C. Burges
- **Description**: Handwritten digits database

### 7. BigANN (SIFT1B, Deep1B)
- **Source**: https://dl.fbaipublicfiles.com/billion-scale-ann-benchmarks
- **Provider**: Facebook AI Research (Meta)
- **License**: Research use only
- **Description**: Billion-scale subsets for ANN algorithm evaluation
- **Citation**: "Billion-scale similarity search with GPUs." Johnson, Douze, and Jegou. IEEE Transactions on Big Data, 2019

### 8. YFCC-10M (Yahoo Flickr Creative Commons)
- **Source**: https://dl.fbaipublicfiles.com/billion-scale-ann-benchmarks/yfcc100M
- **Dimensions**: 192
- **Vectors**: 10M base vectors with metadata
- **License**: Creative Commons licenses (various, as specified by photographers)
- **Provider**: Yahoo / Facebook AI Research
- **Description**: YFCC100M subset with metadata for filtered search benchmarking
- **Citation**: B. Thomee et al. "YFCC100M: The New Data in Multimedia Research." Communications of the ACM, 2016

### 9. Cohere Embeddings
- **Source**: https://huggingface.co/datasets/Cohere/wikipedia-22-12
- **Provider**: Cohere / VectorDBBench
- **Dimensions**: Various (384, 768, 1024)
- **License**: Apache 2.0 (dataset), Terms apply to embeddings
- **Description**: Wikipedia articles embedded with Cohere's embedding models
- **Usage**: Downloaded via VectorDBBench's dataset manager

### 10. OpenAI Embeddings
- **Source**: Generated from https://huggingface.co/datasets/allenai/c4
- **Provider**: VectorDBBench / Allen AI
- **Dimensions**: 1536 (text-embedding-ada-002)
- **License**: ODC-By (C4 dataset), OpenAI API Terms for embeddings
- **Description**: C4 corpus embedded using OpenAI's embedding models
- **Usage**: Downloaded via VectorDBBench's dataset manager

---

## Usage Guidelines

### Datasets
1. **All datasets are used for research and benchmarking purposes only**
2. **No commercial use** of the datasets themselves
3. **Proper attribution** is provided in publications and documentation
4. **No redistribution** - users must download datasets from original sources

### Commercial Use of This Benchmark Tool
- The **benchmark tool itself** (valkey-bench-rs) is BSD-3-Clause licensed
- **You may use this tool commercially** to benchmark your own systems
- **You may NOT commercially redistribute the datasets** - they must be downloaded from official sources
- **Users are responsible** for ensuring their use of datasets complies with original licenses

### How to Cite This Work
If you use valkey-bench-rs in your research, please cite:
```
Zvi Schneider. "valkey-bench-rs: A High-Performance Benchmarking Tool for Vector Search Systems." 2024-2025.
Available at: https://github.com/zvi-code/valkey-bench-rs
```

And cite the relevant datasets used:
- For ANN-Benchmarks datasets, cite: Aumueller et al. "ANN-Benchmarks" (2019)
- For BigANN datasets, cite: Johnson et al. "Billion-scale similarity search with GPUs" (2019)
- For specific datasets (MNIST, GloVe, etc.), see citations above

---

## Dataset License Summary

| Dataset | License | Commercial Use | Attribution Required |
|---------|---------|----------------|---------------------|
| SIFT/GIST | Public Domain (Research) | No | Yes |
| GloVe | PDDL | Yes | Yes |
| Fashion-MNIST | MIT | Yes | Yes |
| MNIST | CC-BY-SA 3.0 | Yes | Yes |
| BigANN | Research Only | No | Yes |
| YFCC-10M | CC (Various) | Depends | Yes |
| Cohere Embeddings | Apache 2.0 | Yes* | Yes |
| OpenAI Embeddings | ODC-By + API Terms | Yes* | Yes |

*Subject to provider's API terms for embeddings

---

## Contact

For licensing questions about this benchmark tool:
- **Author**: Zvi Schneider
- **Repository**: https://github.com/zvi-code/valkey-bench-rs

For dataset licensing questions, please contact the original dataset providers listed above.

---

## Disclaimer

This software is provided "as is" without warranty. Users are responsible for ensuring their use of datasets and tools complies with all applicable licenses and terms of service. The authors make no representations about the suitability of datasets for any particular purpose.

---

*Last Updated: December 2025*
