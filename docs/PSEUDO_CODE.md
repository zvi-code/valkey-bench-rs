
Flow: is a sequence of execution phases
Execution Phase: is multi-threaded execution of commands executed in various threads






valkey-bench-rs Execution Flow - Pseudo Code Breakdown
1. Entry Point and Configuration
FUNCTION main():
    args = parse_cli_arguments()
    
    IF args.cli_mode:
        # Interactive CLI mode (like valkey-cli)
        run_cli_mode(args)
        RETURN
    
    config = BenchmarkConfig.from_cli(args)
    
    # Load dataset if schema-driven format specified
    dataset = NULL
    IF config.schema_path AND config.data_path:
        dataset = DatasetContext.open(schema_path, data_path)
        dataset.apply_cli_overrides(num_vectors, vector_offset)
        config.search_config.dim = dataset.dim()
        config.search_config.distance_metric = dataset.get_metric()
    
    IF config.optimize:
        run_optimization_loop(config, dataset)
        RETURN
    
    orchestrator = Orchestrator.new(config)
    
    IF config.runtime_config_path:
        orchestrator.apply_runtime_config()
    
    base_latency = orchestrator.measure_base_latency()
    print_banner(config, base_latency)
    
    orchestrator.set_dataset(dataset)
    
    # Vector workload setup
    IF has_vector_workload(config.tests):
        setup_vector_index(orchestrator, config)
        orchestrator.build_cluster_tag_map()
    
    IF has_vec_delete_workload(config.tests):
        orchestrator.build_protected_ids()  # Skip ground truth vectors
    
    results = orchestrator.run_all()
    
    export_results(results, config.output_path, config.csv_output)
    print_summary(results)
2. Orchestrator Initialization
CLASS Orchestrator:
    FIELDS:
        config: BenchmarkConfig
        backend: ConnectionBackend          # Cluster or Standalone
        cluster_topology: Option<ClusterTopology>
        dataset: Option<DatasetContext>
        cluster_tag_map: HashMap<u64, String>  # vector_id -> cluster_tag
        protected_ids: HashSet<u64>         # Ground truth IDs (don't delete)
    
    FUNCTION new(config):
        # Step 1: Create initial connection for discovery
        seed_connection = create_connection(config.addresses[0], config.tls, config.auth)
        
        # Step 2: Detect server type and cluster mode
        info_response = seed_connection.command("INFO server")
        engine_type = detect_engine(info_response)  # Valkey, Redis, KeyDB, etc.
        
        cluster_enabled = FALSE
        IF config.cluster_mode:
            cluster_enabled = TRUE
        ELSE:
            # Auto-detect: try CLUSTER INFO
            TRY:
                cluster_info = seed_connection.command("CLUSTER INFO")
                cluster_enabled = parse_cluster_state(cluster_info) == "ok"
            CATCH:
                cluster_enabled = FALSE
        
        # Step 3: Build connection backend
        IF cluster_enabled:
            topology = discover_cluster_topology(seed_connection)
            backend = ClusterBackend.new(topology, config.read_from_replica)
        ELSE:
            backend = StandaloneBackend.new(seed_connection)
        
        RETURN Orchestrator { config, backend, cluster_topology, ... }
3. Cluster Topology Discovery
FUNCTION discover_cluster_topology(seed_connection):
    # Get cluster nodes info
    nodes_output = seed_connection.command("CLUSTER NODES")
    
    topology = ClusterTopology {
        primaries: [],
        replicas: [],
        slot_map: [NULL; 16384]
    }
    
    FOR EACH line IN nodes_output:
        node = parse_node_line(line)
        # Format: <id> <ip:port> <flags> <master-id> <ping> <pong> <epoch> <link-state> <slot-ranges>
        
        IF "master" IN node.flags:
            topology.primaries.append(node)
            FOR slot IN node.slot_ranges:
                topology.slot_map[slot] = node
        ELSE IF "slave" IN node.flags:
            topology.replicas.append(node)
            node.master = find_primary_by_id(node.master_id)
    
    RETURN topology
4. Benchmark Execution - run_all()
FUNCTION Orchestrator.run_all():
    results = []
    
    # Handle composite workloads: "vec-load:10000,vec-query:1000"
    IF config.composite_workload:
        phases = parse_composite(config.composite_workload)
        FOR EACH (workload_type, count) IN phases:
            result = run_single_workload(workload_type, count)
            results.append(result)
        RETURN results
    
    # Handle parallel workloads: "get:80,set:20"
    IF config.parallel_workload:
        result = run_parallel_workload(config.parallel_workload)
        results.append(result)
        RETURN results
    
    # Standard sequential tests
    FOR EACH test_name IN config.tests:
        result = run_single_workload(test_name, config.effective_requests())
        results.append(result)
    
    RETURN results
5. Single Workload Execution
FUNCTION run_single_workload(test_name, total_requests):
    # Step 1: Create workload generator
    workload = create_workload(test_name, config, dataset)
    
    # Step 2: Initialize shared state
    shared_state = SharedBenchmarkState {
        request_counter: AtomicU64(0),           # Global request counter
        completed_counter: AtomicU64(0),         # Completed requests
        error_counter: AtomicU64(0),             # Error count
        total_requests: total_requests,
        rate_limiter: create_rate_limiter(config.requests_per_second),
        stop_flag: AtomicBool(FALSE),
        iteration_strategy: create_iteration_strategy(config),
    }
    
    # Step 3: Distribute clients across threads
    clients_per_thread = config.clients / config.threads
    remainder = config.clients % config.threads
    
    workers = []
    FOR thread_id IN 0..config.threads:
        num_clients = clients_per_thread + (1 IF thread_id < remainder ELSE 0)
        worker = Worker {
            thread_id,
            num_clients,
            shared_state: Arc<shared_state>,
            workload: workload.clone(),
            histogram: ThreadLocalHistogram.new(),  # Lock-free per-thread
            node_histograms: HashMap.new(),         # Per-node metrics
        }
        workers.append(worker)
    
    # Step 4: Spawn worker threads
    handles = []
    start_time = Instant.now()
    
    FOR worker IN workers:
        handle = spawn_thread(|| worker.run())
        handles.append(handle)
    
    # Step 5: Progress reporting (main thread)
    WHILE NOT shared_state.stop_flag.load():
        completed = shared_state.completed_counter.load()
        elapsed = start_time.elapsed()
        throughput = completed / elapsed.as_secs_f64()
        
        IF NOT config.quiet:
            print_progress(test_name, throughput, completed, total_requests)
        
        IF completed >= total_requests:
            shared_state.stop_flag.store(TRUE)
            BREAK
        
        sleep(100ms)
    
    # Step 6: Join threads and merge results
    thread_results = []
    FOR handle IN handles:
        result = handle.join()
        thread_results.append(result)
    
    # Step 7: Merge histograms and metrics
    merged_result = merge_thread_results(thread_results, start_time.elapsed())
    
    RETURN merged_result
6. Worker Thread Execution
CLASS Worker:
    FUNCTION run():
        # Create client connections for this thread
        clients = []
        FOR i IN 0..self.num_clients:
            client = create_client_connection()
            clients.append(client)
        
        # Main work loop
        WHILE NOT self.shared_state.stop_flag.load():
            FOR client IN clients:
                # Claim work atomically
                request_id = self.shared_state.request_counter.fetch_add(1)
                
                IF request_id >= self.shared_state.total_requests:
                    RETURN  # Done
                
                # Rate limiting (if enabled)
                IF self.shared_state.rate_limiter:
                    self.shared_state.rate_limiter.acquire()
                
                # Generate command(s) for this request
                commands = self.workload.generate_commands(request_id, client)
                
                # Execute with timing
                start = Instant.now()
                
                IF config.pipeline > 1:
                    result = execute_pipeline(client, commands, config.pipeline)
                ELSE:
                    result = execute_single(client, commands[0])
                
                latency_us = start.elapsed().as_micros()
                
                # Handle result
                IF result.is_error():
                    self.handle_error(result, client)
                ELSE:
                    self.histogram.record(latency_us)
                    
                    # Per-node histogram for cluster diagnostics
                    node_addr = client.connected_node()
                    self.node_histograms[node_addr].record(latency_us)
                    
                    # Workload-specific result handling (recall, hit/miss, etc.)
                    self.workload.process_result(request_id, result)
                
                self.shared_state.completed_counter.fetch_add(1)
        
        RETURN WorkerResult {
            histogram: self.histogram,
            node_histograms: self.node_histograms,
            workload_stats: self.workload.get_stats(),
        }
7. Command Template System
CLASS CommandTemplate:
    # Pre-encoded RESP command with placeholders
    # Avoids re-encoding on every request
    
    FIELDS:
        base_template: Bytes           # Pre-encoded RESP with placeholders
        placeholders: Vec<Placeholder> # List of (offset, type)
    
    ENUM PlaceholderType:
        KEY,                # Key name (will add prefix + cluster tag)
        VALUE,              # Random or dataset value
        VECTOR,             # Vector bytes from dataset
        FIELD_NAME,         # Hash field name
        NUMERIC_VALUE,      # Generated numeric value
        TAG_VALUE,          # Generated tag value
    
    FUNCTION instantiate(key_id, cluster_tag, dataset, rng):
        # Start with base template
        buffer = self.base_template.clone()
        
        FOR placeholder IN self.placeholders.reverse():  # Reverse to maintain offsets
            value = MATCH placeholder.type:
                KEY => format("{}{{{}}}:{:012d}", prefix, cluster_tag, key_id)
                VALUE => generate_random_bytes(config.data_size)
                VECTOR => dataset.get_vector(key_id)
                FIELD_NAME => placeholder.field_name
                NUMERIC_VALUE => generate_numeric(placeholder.config, key_id, rng)
                TAG_VALUE => generate_tag(placeholder.distribution, rng)
            
            buffer.replace_range(placeholder.offset, value)
        
        RETURN buffer

# Example: SET command template
# Pre-encoded: "*3\r\n$3\r\nSET\r\n${key_len}\r\n{KEY}\r\n${val_len}\r\n{VALUE}\r\n"
8. Workload Implementations
FUNCTION create_workload(test_name, config, dataset):
    MATCH test_name.to_lowercase():
        "ping" => PingWorkload.new()
        
        "get" => KeyValueWorkload.new(
            template: CommandTemplate.parse("GET {KEY}"),
            track_hits: TRUE
        )
        
        "set" => KeyValueWorkload.new(
            template: CommandTemplate.parse("SET {KEY} {VALUE}"),
            track_hits: FALSE
        )
        
        "hset" => HashWorkload.new(
            template: CommandTemplate.parse("HSET {KEY} {FIELD} {VALUE}"),
            address_type: config.address_type  # hash:prefix:field1,field2,...
        )
        
        "vec-load" | "vecload" => VectorLoadWorkload.new(
            template: build_hset_vector_template(config.search_config),
            dataset: dataset,
            tag_field: config.search_config.tag_field,
            tag_distribution: parse_tag_distribution(config.search_tags),
            numeric_fields: config.numeric_field_configs,
            existing_vectors: cluster_tag_map,  # Skip if already exists
        )
        
        "vec-query" | "vecquery" => VectorQueryWorkload.new(
            template: build_ft_search_template(config.search_config),
            dataset: dataset,
            ground_truth: dataset.ground_truth(),
            recall_tracker: RecallTracker.new(),
            tag_filter: config.tag_filter,
            numeric_filters: config.numeric_filters,
        )
        
        "vec-delete" | "vecdelete" => VectorDeleteWorkload.new(
            template: CommandTemplate.parse("DEL {KEY}"),
            protected_ids: protected_ids,  # Don't delete ground truth
        )
        
        _ => error("Unknown workload: {}", test_name)
9. Vector Search Command Templates
FUNCTION build_hset_vector_template(search_config):
    # HSET key:tag:id field1 value1 field2 value2 ... embedding <vector_bytes>
    
    fields = []
    
    # Add tag field if configured
    IF search_config.tag_field:
        fields.append((search_config.tag_field, PlaceholderType.TAG_VALUE))
    
    # Add numeric fields
    FOR numeric_config IN search_config.numeric_fields:
        fields.append((numeric_config.name, PlaceholderType.NUMERIC_VALUE))
    
    # Add vector field (always last for easier parsing)
    fields.append((search_config.vector_field, PlaceholderType.VECTOR))
    
    RETURN CommandTemplate.build_hset(fields)


FUNCTION build_ft_search_template(search_config):
    # FT.SEARCH index_name "filter_expression=>[KNN k @field $BLOB]" 
    #   PARAMS 2 BLOB <vector_bytes> DIALECT 2
    
    # Build filter expression
    filter_parts = []
    
    IF search_config.tag_filter:
        # @category:{electronics|clothing}
        filter_parts.append(format("@{}:{{{}}}", 
            search_config.tag_field, 
            search_config.tag_filter))
    
    FOR numeric_filter IN search_config.numeric_filters:
        # @price:[10 100]
        filter_parts.append(format("@{}:{}", 
            numeric_filter.field, 
            format_range(numeric_filter.min, numeric_filter.max, numeric_filter.inclusive)))
    
    # Build KNN part
    knn_expr = format("[KNN {} @{} $BLOB AS score]", 
        search_config.k, 
        search_config.vector_field)
    
    IF filter_parts.is_empty():
        query = "*=>" + knn_expr
    ELSE:
        query = "(" + filter_parts.join(" ") + ")=>" + knn_expr
    
    RETURN CommandTemplate {
        command: "FT.SEARCH",
        args: [
            search_config.index_name,
            query,
            "PARAMS", "2", "BLOB", PlaceholderType.VECTOR,
            "SORTBY", "score",
            "DIALECT", "2",
            "LIMIT", "0", str(search_config.k)
        ],
        nocontent: search_config.nocontent,
    }
10. Iteration Strategies
FUNCTION create_iteration_strategy(config):
    MATCH config.iteration:
        "sequential" | "seq" => 
            SequentialIterator { current: 0 }
        
        "random" | "random:SEED" =>
            RandomIterator { 
                rng: SplitMix64.seed(config.seed),
                keyspace: config.keyspace_len 
            }
        
        "subset:START:END" =>
            SubsetIterator { 
                start: START, 
                end: END,
                rng: SplitMix64.seed(config.seed)
            }
        
        "zipfian:SKEW" | "zipfian:SKEW:SEED" =>
            ZipfianIterator {
                skew: SKEW,
                keyspace: config.keyspace_len,
                rng: SplitMix64.seed(SEED or config.seed),
                precomputed_cdf: precompute_zipfian_cdf(SKEW, keyspace)
            }


CLASS SequentialIterator:
    FUNCTION next_key(request_id):
        RETURN request_id % self.keyspace

CLASS RandomIterator:
    FUNCTION next_key(request_id):
        # Deterministic: same seed + request_id = same key
        # Uses global atomic counter for consistency across threads
        mixed = splitmix64(self.seed ^ request_id)
        RETURN mixed % self.keyspace

CLASS ZipfianIterator:
    FUNCTION next_key(request_id):
        # Power-law distribution: few keys get most traffic
        u = self.rng.next_f64()  # Uniform [0, 1)
        RETURN binary_search(self.precomputed_cdf, u)
11. Cluster Routing and MOVED Handling
CLASS ClusterBackend:
    FUNCTION route_command(key, command):
        # Calculate slot from key (with hash tag support)
        slot = crc16(extract_hash_key(key)) % 16384
        
        # Get target node based on read/write and RFR strategy
        IF command.is_read() AND self.read_from_replica != "primary":
            node = select_replica_for_slot(slot)
        ELSE:
            node = self.slot_map[slot]
        
        RETURN node
    
    FUNCTION execute_with_redirect(client, command):
        MAX_REDIRECTS = 5
        
        FOR attempt IN 0..MAX_REDIRECTS:
            result = client.execute(command)
            
            IF result.is_moved():
                # MOVED 3999 127.0.0.1:6381
                (slot, new_addr) = parse_moved(result)
                self.update_slot_mapping(slot, new_addr)
                client.reconnect_to(new_addr)
                CONTINUE
            
            IF result.is_ask():
                # ASK 3999 127.0.0.1:6381 (slot migration in progress)
                (slot, new_addr) = parse_ask(result)
                client.send_asking(new_addr)
                CONTINUE
            
            IF result.is_clusterdown():
                sleep(100ms)
                self.refresh_topology()
                CONTINUE
            
            RETURN result
        
        RETURN Error("Max redirects exceeded")


FUNCTION extract_hash_key(key):
    # Support hash tags: "user:{abc}:profile" -> "abc"
    IF "{" IN key AND "}" IN key:
        start = key.index("{") + 1
        end = key.index("}")
        IF start < end:
            RETURN key[start..end]
    RETURN key
12. Metrics Collection and Merging
CLASS ThreadLocalHistogram:
    # HdrHistogram-style with lock-free recording
    # Range: 1us to 10s with 3 significant figures
    
    FIELDS:
        counts: [AtomicU64; NUM_BUCKETS]
        total_count: AtomicU64
        total_sum: AtomicU64
        min: AtomicU64
        max: AtomicU64
    
    FUNCTION record(latency_us):
        bucket = compute_bucket(latency_us)
        self.counts[bucket].fetch_add(1)
        self.total_count.fetch_add(1)
        self.total_sum.fetch_add(latency_us)
        atomic_min(&self.min, latency_us)
        atomic_max(&self.max, latency_us)
    
    FUNCTION percentile(p):
        target = (p / 100.0) * self.total_count.load()
        cumulative = 0
        FOR bucket IN 0..NUM_BUCKETS:
            cumulative += self.counts[bucket].load()
            IF cumulative >= target:
                RETURN bucket_to_value(bucket)
        RETURN self.max.load()


FUNCTION merge_thread_results(thread_results, duration):
    merged_histogram = ThreadLocalHistogram.new()
    merged_node_histograms = HashMap.new()
    merged_recall = RecallStats.new()
    merged_keyspace = KeyspaceStats.new()
    
    FOR result IN thread_results:
        merged_histogram.merge(result.histogram)
        
        FOR (node, hist) IN result.node_histograms:
            merged_node_histograms[node].merge(hist)
        
        merged_recall.merge(result.workload_stats.recall)
        merged_keyspace.merge(result.workload_stats.keyspace)
    
    total_ops = merged_histogram.total_count.load()
    throughput = total_ops / duration.as_secs_f64()
    
    RETURN BenchmarkResult {
        test_name,
        throughput,
        total_ops,
        error_count: sum(r.error_count for r in thread_results),
        duration,
        histogram: merged_histogram,
        node_histograms: merged_node_histograms,
        recall_stats: merged_recall,
        keyspace_stats: merged_keyspace,
    }
13. Recall Computation (Vector Search)
CLASS RecallTracker:
    FIELDS:
        total_queries: AtomicU64
        total_recall_sum: AtomicF64  # Sum of individual query recalls
        per_query_recalls: Vec<f64>  # For detailed analysis
    
    FUNCTION compute_recall(query_id, returned_ids, ground_truth):
        # ground_truth[query_id] = [id1, id2, ..., id_k] (sorted by distance)
        expected = Set(ground_truth[query_id][0..k])
        actual = Set(returned_ids[0..k])
        
        intersection = expected.intersection(actual).len()
        recall = intersection / k
        
        self.total_queries.fetch_add(1)
        self.total_recall_sum.fetch_add(recall)
        
        RETURN recall
    
    FUNCTION average_recall():
        RETURN self.total_recall_sum.load() / self.total_queries.load()
14. Parallel Workload Execution
FUNCTION run_parallel_workload(parallel_spec):
    # Parse "get:80,set:20" into weighted workloads
    workloads = []
    total_weight = 0
    
    FOR part IN parallel_spec.split(","):
        (name, weight) = part.split(":")
        workloads.append((name, parse_int(weight)))
        total_weight += parse_int(weight)
    
    # Normalize weights to cumulative distribution
    cumulative = []
    running_sum = 0
    FOR (name, weight) IN workloads:
        running_sum += weight
        cumulative.append((name, running_sum / total_weight))
    
    # Create composite workload selector
    FUNCTION select_workload(rng):
        r = rng.next_f64()  # [0, 1)
        FOR (name, threshold) IN cumulative:
            IF r < threshold:
                RETURN name
        RETURN cumulative.last().name
    
    # Run with workload selection per request
    # Each worker randomly selects workload type per request based on weights
    ...
15. Optimization Loop
FUNCTION run_optimization_loop(config, dataset):
    objectives = Objectives.parse(config.optimize_objective)
    constraints = [Constraint.parse(c) for c in config.optimize_constraints]
    parameters = [TunableParameter.parse(p) for p in config.optimize_parameters]
    
    optimizer = Optimizer.new(objectives, constraints, parameters)
    
    # Create base orchestrator (reuse topology for efficiency)
    base_orchestrator = Orchestrator.new(config)
    setup_vector_index(base_orchestrator, config)
    
    shared_topology = base_orchestrator.cluster_topology()
    shared_tag_map = base_orchestrator.cluster_tag_map()
    
    iteration = 0
    WHILE optimizer.has_next():
        test_config = optimizer.next_config()
        iteration += 1
        
        # Apply test parameters to config
        run_config = config.clone()
        run_config.clients = test_config.clients OR config.clients
        run_config.threads = test_config.threads OR config.threads
        run_config.pipeline = test_config.pipeline OR config.pipeline
        run_config.search_config.ef_search = test_config.ef_search
        run_config.requests = optimizer.recommended_requests()  # Adaptive duration
        
        # Reuse topology (avoid port exhaustion from rediscovery)
        iter_orchestrator = Orchestrator.with_topology(run_config, shared_topology)
        iter_orchestrator.set_cluster_tag_map(shared_tag_map)
        
        result = iter_orchestrator.run_all()[0]
        
        optimizer.record_result(test_config, result)
        
        print_iteration_summary(iteration, optimizer.phase(), test_config, result)
    
    print_optimization_summary(optimizer)
    print_recommended_command(optimizer.best_result())
16. Dataset Memory Mapping
CLASS DatasetContext:
    FIELDS:
        schema: Schema                    # Parsed YAML schema
        mmap: MemoryMappedFile           # Zero-copy file mapping
        vector_offset: usize             # Offset to vector data section
        query_offset: usize              # Offset to query vectors
        ground_truth_offset: usize       # Offset to neighbor IDs
        effective_num_vectors: u64       # May be limited by CLI args
        effective_vector_offset: u64     # Starting vector index
    
    FUNCTION open(schema_path, data_path):
        schema = parse_yaml(schema_path)
        mmap = MemoryMappedFile.open(data_path, MADV_WILLNEED)  # Prefetch
        
        # Calculate section offsets from schema
        vector_offset = 0
        query_offset = schema.records.count * schema.record_size
        ground_truth_offset = query_offset + schema.queries.count * schema.record_size
        
        RETURN DatasetContext { schema, mmap, ... }
    
    FUNCTION get_vector(index):
        # Zero-copy access to vector bytes
        actual_index = self.effective_vector_offset + index
        offset = self.vector_offset + actual_index * self.dim * sizeof(f32)
        RETURN self.mmap.slice(offset, self.dim * sizeof(f32))
    
    FUNCTION get_query(index):
        offset = self.query_offset + index * self.dim * sizeof(f32)
        RETURN self.mmap.slice(offset, self.dim * sizeof(f32))
    
    FUNCTION ground_truth(query_index):
        # Returns array of neighbor IDs for recall computation
        offset = self.ground_truth_offset + query_index * k * sizeof(u64)
        RETURN self.mmap.read_u64_array(offset, k)
17. Index Creation and Management
FUNCTION setup_vector_index(orchestrator, config):
    IF NOT has_vector_workload(config.tests):
        RETURN
    
    IF config.dropindex:
        IF orchestrator.search_index_exists():
            orchestrator.drop_search_index()
    
    IF NOT orchestrator.search_index_exists():
        orchestrator.create_search_index()
        sleep(10s)  # Wait for propagation across cluster
    
    orchestrator.wait_for_search_indexing(timeout=300s)


FUNCTION create_search_index():
    # Build FT.CREATE command
    # FT.CREATE idx ON HASH PREFIX 1 vec: SCHEMA 
    #   embedding VECTOR HNSW 6 TYPE FLOAT32 DIM 768 DISTANCE_METRIC COSINE
    #   category TAG
    #   price NUMERIC
    
    args = ["FT.CREATE", config.search_config.index_name]
    args.extend(["ON", "HASH", "PREFIX", "1", config.search_config.prefix])
    args.append("SCHEMA")
    
    # Vector field
    args.extend([
        config.search_config.vector_field, "VECTOR", 
        config.search_config.algorithm,  # HNSW or FLAT
        "6",  # Number of HNSW params
        "TYPE", "FLOAT32",
        "DIM", str(config.search_config.dim),
        "DISTANCE_METRIC", config.search_config.distance_metric,
    ])
    
    IF config.search_config.algorithm == "HNSW":
        args.extend([
            "M", str(config.search_config.hnsw_m),
            "EF_CONSTRUCTION", str(config.search_config.ef_construction),
        ])
    
    # Tag field
    IF config.search_config.tag_field:
        args.extend([config.search_config.tag_field, "TAG"])
    
    # Numeric fields
    FOR numeric_field IN config.search_config.numeric_fields:
        args.extend([numeric_field.name, "NUMERIC"])
    
    send_cluster_broadcast(args)  # Send to all primaries

Summary: Key Architectural Decisions
ComponentDesign ChoiceRationaleThread ModelN workers × M clients/workerBalances parallelism with connection overheadCountersAtomic fetch_add for request claimingLock-free, no contentionHistogramsThread-local, merged at endZero contention during hot pathCommand TemplatesPre-encoded RESP with placeholdersAvoid encoding overhead per requestCluster RoutingCRC16 slot map with hash tag supportO(1) routing, proper shardingDataset AccessMemory-mapped filesZero-copy, OS page cacheIterationDeterministic (seed-based)Reproducible benchmarksRate LimitingToken bucket per-threadControlled load generation