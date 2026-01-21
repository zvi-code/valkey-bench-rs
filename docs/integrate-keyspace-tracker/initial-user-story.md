## Initial user story [free discussion text - reflexts what the user wants to achieve]
review the keyspace_tracker. It represent key space build of groups of keys that share prefix. 
Key format is <prefix><12 bytes padded number in range of min-max>. Hash keys can have Key format is <prefix><12 bytes padded number in range of min-max(prefix)>.<field-prefix><12 bytes padded number in range of min-max(field-prefix)> 
The main use case for it is to build benchmarking logic that operate on keyspace with some pattern in predictable consistent way with very performant iterator. Keys are associated to key prefix and followed by number in the range defined for the Prefix (can be extended\trimmed) and for hash keys can have fields also defined in the same format prefix with 
Some examples:
1. Iterate key space sequentially or semi-sequentially (splitted into non-overlapping covering subset of the space assigned to threads and each thread is iterating sequentially)
* + set (to mark existence) in case of write operation
*  
   * Iterate key sub-space (defined by offset+length)
1. same as #1 but random of different randomness properties
2. #1 & #2 but with overlap between threads
3. Iterate by existence - like run on existing\non-existing keys
   1. iterate over x% existing and 100-x% non-existing
Examples of benchmarking load that will be using this:
* prefill db with 10M keys sequentially (single\multi-threaded)
* overwrite randomly 50% higher portion of a key prefix
* fill randomly 1M hash keys each have random number of fields with max number of fields for single hash is 10M and min is 1 and avg is 10
* prefix key space of size 100M + random overwrite + delete randomly 50% of the space + random get with 100% hit + delete 50% of remaining + write non-existing to get back to 100M keys
* Set keyspace with 10% new writes and 90% overwrite

The design of this keyspace_tracker was to support exctly these capabilities. Review the code and evaluate how to implement the capabilities using the keyspace_tracker. If you find gaps in the implementation, list them for my review

  be mindful of the multi-threaded and performance requirements. Do not create code duplication with similar functionality, prioritize existing code object\struct and extend as needed

  Consider the example of workload we want to support:

1. Iterate key space sequentially or semi-sequentially (splitted into non-overlapping covering subset of the space assigned to threads and each thread is iterating sequentially)

1.1 + set (to mark existence) in case of write operation

1.2. Iterate key sub-space (defined by offset+length)
2. same as #1 but random of different randomness properties
3. #1 & #2 but with overlap between threads
4. Iterate only existing keys (for read\delete operations)
5. Iterate only non-existing keys (for write operations)
6. Iterate hierarchical key space (hash keys with fields)
7. Atomic claim of key (to avoid double write from multiple threads)
8. Group level operations (claim\iterate over groups of keys that share same prefix)
9. Iterate by existence - like run on existing\non-existing keys
10. iterate over x% existing and 100-x% non-existing
And extend locigally to simulate valkey user workload, that have session store, or use ttl to retire old entries, or have appends to keys and trim, or that have max memory configured, or that have various mixed pattern workloads, for example, they might have random overrite and semi-sequentila read and so on. Another major use case is for testing defrag capabilities of valkey

more use cases:
We initialize vector dataset from a public source, than we have query vectors and ground truth vectors. After running different workloads that made changes, possibly deleted vectors
The benchmark is running again, initially it will issue a scan process will scan the engine and find all existing keys
When benchmark starts we want to:
1. fill back in any missing keys 
2. fill back only keys that are part of ground-truth of a query vector (so we can calculate recall)
3. no backfill, just be able to say if a ground-truth vector exists
than we will run some workload on the entire dataset space or on the existing with various patterns


note, recall calculation is not in the responsibility of the tracker. For all these examples, the keyspace_tracker only needs to provide the appropriate tools\interface to generate efficiently key patterns (over whatever space) that will be used by the application to implement the use case. So for example, the knowledge of what are the keys and what they are used for is not part of keyspace_tracker responsibility.
For example: the application will create group of keys (say prefixed with query-vectors) that are marked as query vectors and each such a hash key with k elements holding the vector-id of the ground-truth vector for every query vector

