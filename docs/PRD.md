# Product Requirements Document: BTreeDB

## 1. Introduction

### 1.1 Document Purpose
This Product Requirements Document (PRD) defines the specifications, features, and requirements for BTreeDB - a clean, educational implementation of a persistent B-Tree database in Rust. This document serves as the single source of truth for understanding what the product should accomplish, its target audience, and the technical constraints that guide its development.

### 1.2 Project Overview
BTreeDB is a key-value database that implements a B-Tree data structure with on-disk persistence. The project is designed as an educational tool to demonstrate core database concepts including:
- Page-based storage architecture
- B-Tree indexing for efficient lookups
- Persistent storage with file I/O
- Interactive command-line interface

The database provides a simple yet complete implementation that can store and retrieve string key-value pairs with guaranteed persistence across sessions.

### 1.3 Objectives
- **Educational**: Provide a clear, readable implementation that teaches database internals
- **Functional**: Deliver a working key-value store with persistence
- **Performant**: Achieve O(log n) search and insert operations
- **Reliable**: Ensure data durability and integrity
- **Usable**: Offer an intuitive REPL interface for interaction

## 2. Target Audience

### 2.1 Primary Users

**1. Students and Learners**
- **Needs**: Understanding how databases work internally
- **Goals**: Learn B-Tree algorithms, page-based storage, and serialization
- **Technical Level**: Intermediate to advanced Rust developers

**2. Educators**
- **Needs**: Teaching material for database systems courses
- **Goals**: Demonstrate database concepts with working code
- **Technical Level**: Advanced

**3. Developers Building Database Systems**
- **Needs**: Reference implementation for B-Tree structures
- **Goals**: Understand practical implementation details
- **Technical Level**: Advanced

### 2.2 User Personas

**Persona 1: "The Student" - Alex**
- Computer science student learning about databases
- Wants to understand how SQLite or PostgreSQL work internally
- Needs clear, well-commented code
- Values educational documentation and examples

**Persona 2: "The Educator" - Dr. Smith**
- University professor teaching database systems
- Needs a complete, working example to show students
- Values correctness and clarity over performance optimizations
- Uses the project as a teaching tool in lectures

**Persona 3: "The Hacker" - Jordan**
- Experienced developer building a custom database
- Needs a reference for B-Tree implementation patterns
- Values code quality and architectural decisions
- May fork and extend the project

## 3. Use Cases

### 3.1 Core Use Cases

**UC1: Store and Retrieve Data**
- **Actor**: User
- **Precondition**: Database file exists or can be created
- **Flow**:
  1. User starts the REPL
  2. User inserts key-value pairs using `set` command
  3. User retrieves values using `get` command
  4. User exits, data persists to disk
- **Postcondition**: All data is safely stored and can be retrieved in future sessions

**UC2: Learn Database Internals**
- **Actor**: Student/Developer
- **Precondition**: User has Rust installed and can read code
- **Flow**:
  1. User reads the codebase to understand B-Tree structure
  2. User traces an insert operation through the code
  3. User observes node splitting behavior
  4. User examines page serialization format
- **Postcondition**: User understands how B-Trees work in practice

**UC3: Run Performance Benchmarks**
- **Actor**: Developer/Researcher
- **Precondition**: Database is built and benchmark suite is available
- **Flow**:
  1. User runs benchmark suite
  2. System measures throughput, latency, and efficiency
  3. User reviews generated HTML reports
  4. User compares performance across different scenarios
- **Postcondition**: Performance metrics are documented

**UC4: Integration Testing**
- **Actor**: Developer
- **Precondition**: Test suite is available
- **Flow**:
  1. Developer runs integration tests
  2. System verifies correctness of operations
  3. System verifies data persistence
  4. System verifies edge cases (splits, root growth)
- **Postcondition**: All tests pass, confirming system correctness

## 4. Features and Functionality

### 4.1 Core Features

#### F1: Key-Value Storage
- **Description**: Store arbitrary string key-value pairs
- **Priority**: P0 (Must Have)
- **Acceptance Criteria**:
  - Can store at least 1000 key-value pairs
  - Supports keys and values up to 1KB each
  - Updates existing keys when inserting duplicates
  - Returns `(nil)` for non-existent keys

#### F2: Persistent Storage
- **Description**: All data persists to disk across sessions
- **Priority**: P0 (Must Have)
- **Acceptance Criteria**:
  - Data survives process termination
  - Data survives system restarts
  - Database file format is stable and readable
  - Magic bytes verify file integrity

#### F3: B-Tree Indexing
- **Description**: Use B-Tree structure for efficient lookups
- **Priority**: P0 (Must Have)
- **Acceptance Criteria**:
  - Search operations are O(log n) complexity
  - Insert operations are O(log n) complexity
  - Tree maintains balance through automatic splitting
  - Tree height grows logarithmically with data size

#### F4: Interactive REPL
- **Description**: Command-line interface for database operations
- **Priority**: P0 (Must Have)
- **Acceptance Criteria**:
  - Supports `set <key> <value>` command
  - Supports `get <key>` command
  - Supports `.exit` command with data flush
  - Provides command history (via rustyline)
  - Shows clear error messages

#### F5: Node Splitting
- **Description**: Automatic splitting when nodes exceed capacity
- **Priority**: P0 (Must Have)
- **Acceptance Criteria**:
  - Leaf nodes split when exceeding MAX_LEAF_KEYS
  - Internal nodes split when exceeding MAX_INTERNAL_KEYS
  - Root node splitting creates new root level
  - Splits maintain tree balance

### 4.2 Secondary Features

#### F6: Database Header
- **Description**: Metadata stored in page 0
- **Priority**: P1 (Should Have)
- **Acceptance Criteria**:
  - Contains magic bytes for file identification
  - Tracks root page ID
  - Provides reserved space for future use
  - Header is validated on database open

#### F7: Page-Based Architecture
- **Description**: Fixed-size 4KB pages for storage
- **Priority**: P1 (Should Have)
- **Acceptance Criteria**:
  - All pages are exactly 4096 bytes
  - Pages are read/written atomically
  - Page I/O is abstracted through Pager interface
  - Pages support efficient random access

#### F8: Comprehensive Testing
- **Description**: Test suite covering core functionality
- **Priority**: P1 (Should Have)
- **Acceptance Criteria**:
  - Integration tests for basic operations
  - Tests verify persistence across sessions
  - Tests verify node splitting behavior
  - Tests handle edge cases

#### F9: Performance Benchmarks
- **Description**: Benchmark suite for performance analysis
- **Priority**: P2 (Nice to Have)
- **Acceptance Criteria**:
  - Measures write throughput
  - Measures lookup latency
  - Measures storage efficiency
  - Generates HTML reports

### 4.3 Future Features (Out of Scope for MVP)

- Delete operations
- Range queries (scan operations)
- Transaction support with rollback
- WAL (Write-Ahead Logging)
- Compression for values
- Multiple database support
- Backup and restore functionality
- Concurrent access support
- Different value types (not just Strings)

## 5. Technical Requirements

### 5.1 Architecture Requirements

#### 5.1.1 System Architecture
- **Language**: Rust (stable toolchain)
- **Storage Model**: Single-file, page-based storage
- **Data Structure**: B-Tree with configurable branching factor
- **I/O Model**: Synchronous file I/O with explicit flushing

#### 5.1.2 Component Architecture

**Pager Module** (`src/pager.rs`)
- Manages file I/O operations
- Handles 4KB page reads/writes
- Provides abstraction over file system
- Supports random access to pages

**Node Module** (`src/node.rs`)
- Defines B-Tree node structures (Leaf and Internal)
- Handles serialization/deserialization
- Manages node type identification
- Enforces node size constraints

**BTree Module** (`src/btree.rs`)
- Implements B-Tree algorithms
- Handles insert, search, and split operations
- Manages root page tracking
- Coordinates between Pager and Node modules

**Main Module** (`src/main.rs`)
- Provides REPL interface
- Parses user commands
- Coordinates database operations
- Handles graceful shutdown

### 5.2 Data Format Requirements

#### 5.2.1 Page Layout
- **Page Size**: 4096 bytes (4KB)
- **Page 0**: Database header (first 100 bytes)
  - Bytes 0-6: Magic bytes "BTREEDB"
  - Bytes 7-10: Root page ID (u32, little-endian)
  - Bytes 11-99: Reserved for future use
- **Page 1+**: B-Tree nodes

#### 5.2.2 Node Serialization Format
- **Byte 0**: Node type (0 = Leaf, 1 = Internal)
- **Bytes 1-4**: Number of keys (u32, little-endian)
- **Leaf Node Data**:
  - For each key-value pair:
    - 4 bytes: Key length (u32, little-endian)
    - N bytes: Key data (UTF-8)
    - 4 bytes: Value length (u32, little-endian)
    - M bytes: Value data (UTF-8)
- **Internal Node Data**:
  - For each key:
    - 4 bytes: Key length (u32, little-endian)
    - N bytes: Key data (UTF-8)
  - For each child (num_keys + 1 children):
    - 4 bytes: Page ID (u32, little-endian)
- **Remainder**: Zero-padded to exactly 4096 bytes

#### 5.2.3 Node Capacity Constraints
- **MAX_LEAF_KEYS**: 3 (configurable, reduced to support 1KB values)
- **MAX_INTERNAL_KEYS**: 10 (configurable)
- Nodes must fit within single 4KB page
- Serialization must validate size constraints

### 5.3 Performance Requirements

#### 5.3.1 Time Complexity
- **Search**: O(log n) where n is number of keys
- **Insert**: O(log n) where n is number of keys
- **Split**: O(1) amortized (splits occur infrequently)

#### 5.3.2 Space Complexity
- **Storage Overhead**: Minimal (only page padding)
- **Memory Usage**: O(height) for recursive operations
- **File Size**: Approximately (number of pages) × 4KB

#### 5.3.3 Benchmark Targets
- **Write Throughput**: Measure writes per second at various scales
- **Lookup Latency**: Compare B-Tree vs linear scan performance
- **Storage Efficiency**: Measure overhead percentage
- **Recovery Time**: Measure database load time

### 5.4 Reliability Requirements

#### 5.4.1 Data Durability
- All writes must be flushed to disk on `.exit` command
- File system sync ensures data survives crashes
- Magic bytes verify database file integrity
- Header validation prevents corruption

#### 5.4.2 Error Handling
- Graceful handling of file I/O errors
- Validation of serialization/deserialization
- Clear error messages for user-facing operations
- Proper error propagation through call stack

#### 5.4.3 Data Integrity
- Node size constraints prevent page overflow
- B-Tree invariants maintained (balanced tree)
- Root page ID always points to valid node
- Magic bytes prevent accidental file corruption

### 5.5 Compatibility Requirements

#### 5.5.1 Platform Support
- **Primary**: Unix-like systems (Linux, macOS)
- **Secondary**: Windows (if file I/O is compatible)
- **Architecture**: Little-endian (for serialization)

#### 5.5.2 Rust Version
- **Minimum**: Rust 1.70.0 (stable)
- **Recommended**: Latest stable Rust
- **Toolchain**: Standard rustup installation

#### 5.5.3 Dependencies
- **Runtime**: `byteorder`, `rustyline`
- **Development**: `tempfile`, `criterion`
- **Build**: Standard Cargo toolchain

## 6. User Interface Requirements

### 6.1 Command-Line Interface

#### 6.1.1 REPL Interface
- **Prompt**: `btreedb> `
- **Command History**: Supported via rustyline
- **Input Method**: Line-based input
- **Output Format**: Plain text responses

#### 6.1.2 Commands

**SET Command**
- **Syntax**: `set <key> <value>`
- **Description**: Insert or update a key-value pair
- **Response**: `OK` on success
- **Error Handling**: Display error message on failure

**GET Command**
- **Syntax**: `get <key>`
- **Description**: Retrieve value for a key
- **Response**: Value string, or `(nil)` if not found
- **Error Handling**: Display error message on failure

**EXIT Command**
- **Syntax**: `.exit`
- **Description**: Exit REPL and flush all data to disk
- **Response**: `All data flushed to disk. Goodbye!`
- **Side Effects**: Calls `sync_all()` on database file

#### 6.1.3 User Experience
- Clear command syntax documentation on startup
- Immediate feedback for all operations
- Graceful error messages (no stack traces)
- Command history for convenience

## 7. Success Metrics

### 7.1 Functional Metrics
- **Correctness**: 100% test pass rate
- **Persistence**: Data survives 100% of normal shutdowns
- **Capacity**: Successfully stores 1000+ key-value pairs
- **Reliability**: Zero data corruption in normal operation

### 7.2 Performance Metrics
- **Search Performance**: O(log n) confirmed through benchmarks
- **Insert Performance**: O(log n) confirmed through benchmarks
- **Storage Efficiency**: < 20% overhead (excluding page padding)
- **Recovery Time**: < 100ms for databases up to 10MB

### 7.3 Educational Metrics
- **Code Readability**: Clear, well-commented implementation
- **Documentation**: Complete README with examples
- **Learnability**: Users can understand core concepts from code
- **Extensibility**: Code structure supports future enhancements

## 8. Constraints and Limitations

### 8.1 Technical Constraints
- **Single-threaded**: No concurrent access support
- **String-only**: Keys and values must be UTF-8 strings
- **Single-file**: Database stored in single file
- **Fixed page size**: 4KB pages (not configurable)
- **No transactions**: No rollback or atomicity guarantees
- **No WAL**: Direct page writes (no write-ahead logging)

### 8.2 Scalability Constraints
- **File size**: Limited by file system (typically 2GB+)
- **Key size**: Maximum ~1KB per key (due to page constraints)
- **Value size**: Maximum ~1KB per value (due to page constraints)
- **Tree height**: Grows logarithmically (typically < 10 levels for millions of keys)

### 8.3 Operational Constraints
- **No backup**: No built-in backup/restore functionality
- **No replication**: Single database instance only
- **No network**: Local file access only
- **No encryption**: Data stored in plain text

## 9. Development Requirements

### 9.1 Code Quality
- **Formatting**: `cargo fmt` compliant
- **Linting**: `cargo clippy` with `-D warnings`
- **Documentation**: Public APIs must be documented
- **Testing**: Integration tests for core functionality

### 9.2 Continuous Integration
- **Automated Testing**: All tests run on push to main
- **Code Quality Checks**: Format and lint checks
- **Rust Version**: Tested on stable Rust
- **Platform**: Tested on Linux and macOS

### 9.3 Documentation Requirements
- **README**: Complete usage and architecture documentation
- **Code Comments**: Explain complex algorithms
- **Examples**: Working examples in README
- **Architecture**: Document design decisions

## 10. Timeline and Milestones

### 10.1 MVP (Minimum Viable Product) - ✅ Complete
- [x] Core B-Tree implementation
- [x] Page-based storage
- [x] REPL interface
- [x] Basic persistence
- [x] Integration tests

### 10.2 Enhanced Features - ✅ Complete
- [x] Performance benchmarks
- [x] Comprehensive documentation
- [x] CI/CD pipeline
- [x] Error handling improvements

### 10.3 Phase 1: Delete Operations - ✅ Complete
- [x] Add `delete` method to BTree (`src/btree.rs`)
- [x] Implement key removal from leaf nodes
- [x] Handle root demotion when root becomes empty
- [x] Add `delete <key>` command to REPL
- [x] Integration tests for deletion

### 10.4 Phase 2: Cursor and Range Queries - ✅ Complete
- [x] Create `Cursor` struct for tree traversal (`src/cursor.rs`)
- [x] Implement cursor navigation: `seek`, `next`, `seek_first`
- [x] Implement `scan_range(start, end)` for range queries
- [x] Add `scan [start] [end]` command to REPL
- [x] Unit tests for cursor operations

### 10.5 Phase 3: Database Statistics and Debugging - ✅ Complete
- [x] Track statistics: key count, tree height, page count, node counts
- [x] Add `.stats` command to REPL
- [x] Add `.dump` command for tree visualization
- [x] Implement `DatabaseStats` struct and collection methods

### 10.6 Phase 4: Multiple Value Types - ✅ Complete
- [x] Create `Value` enum with type tag (String, Integer, Float, Binary, Null)
- [x] Implement serialization/deserialization with type prefix byte
- [x] Add parsing support for type prefixes (`i:`, `f:`, `b:`, `s:`, `null`)
- [x] Unit tests for value operations

### 10.7 Phase 5: Write-Ahead Logging (WAL) - ✅ Complete
- [x] Create WAL file (`*.db-wal`) alongside main database
- [x] Log all page modifications with checksums
- [x] Implement checkpoint mechanism to truncate WAL
- [x] Implement recovery module for replaying WAL on startup
- [x] Unit tests for WAL operations

### 10.8 Phase 6: Transaction Support - ✅ Complete
- [x] Implement `Transaction` struct with begin/commit/rollback
- [x] Create `TransactionManager` for coordinating transactions
- [x] Add savepoints for nested transaction support
- [x] Unit tests for transaction operations

### 10.9 Phase 7: Value Compression - ✅ Complete
- [x] Implement RLE compression for educational purposes
- [x] Add compression flag and threshold support
- [x] Create `CompressedData` struct with serialization
- [x] Add `CompressionStats` for tracking compression effectiveness
- [x] Unit tests for compression operations

### 10.10 Phase 8: Backup and Restore - ✅ Complete
- [x] Implement backup/restore functions with WAL support
- [x] Add backup verification functionality
- [x] Create `BackupInfo` struct for backup metadata
- [x] Unit tests for backup operations

### 10.11 Phase 9: Multiple Database Support - ✅ Complete
- [x] Create `DatabaseManager` to handle multiple instances
- [x] Add `DatabaseConfig` for database configuration
- [x] Implement `DatabaseHandle` for managed database access
- [x] Unit tests for multi-database operations

### 10.12 Phase 10: Concurrent Access - ✅ Complete
- [x] Implement `PageLock` with read-write semantics
- [x] Create `LockManager` for page-level locking
- [x] Add `ConnectionPool` for connection management
- [x] Unit tests for concurrent access patterns

### 10.13 Implementation Priority Matrix

| Phase | Feature | Complexity | Dependencies | Educational Value |
|-------|---------|------------|--------------|-------------------|
| 1 | Delete | Medium | None | High |
| 2 | Cursors/Range | Medium | None | High |
| 3 | Statistics | Low | None | Medium |
| 4 | Value Types | Medium | None | Medium |
| 5 | WAL | High | None | Very High |
| 6 | Transactions | High | Phase 5 | Very High |
| 7 | Compression | Low | Phase 4 | Low |
| 8 | Backup | Medium | Phase 5 | Medium |
| 9 | Multi-DB | Low | None | Low |
| 10 | Concurrency | Very High | Phase 5, 6 | Very High |

## 11. Risks and Mitigation

### 11.1 Technical Risks

**Risk 1: Data Corruption**
- **Probability**: Low
- **Impact**: High
- **Mitigation**: Magic bytes validation, header checks, comprehensive testing

**Risk 2: Performance Degradation**
- **Probability**: Medium
- **Impact**: Medium
- **Mitigation**: Benchmarking, performance profiling, algorithmic optimization

**Risk 3: Platform Compatibility**
- **Probability**: Low
- **Impact**: Low
- **Mitigation**: CI testing on multiple platforms, standard Rust APIs

### 11.2 Educational Risks

**Risk 4: Code Complexity**
- **Probability**: Medium
- **Impact**: Medium
- **Mitigation**: Clear documentation, code comments, architectural simplicity

**Risk 5: Incomplete Examples**
- **Probability**: Low
- **Impact**: Low
- **Mitigation**: Comprehensive README, working examples, test cases

## 12. Dependencies and Assumptions

### 12.1 External Dependencies
- **Rust Standard Library**: File I/O, collections, error handling
- **byteorder**: Binary serialization
- **rustyline**: REPL interface
- **criterion**: Benchmarking (development)
- **tempfile**: Testing utilities (development)

### 12.2 System Assumptions
- File system supports random access reads/writes
- Sufficient disk space for database file
- UTF-8 encoding for all string data
- Little-endian byte order for serialization

### 12.3 User Assumptions
- Users have Rust installed and can build the project
- Users understand basic command-line interfaces
- Users are familiar with key-value store concepts
- Users can read Rust code (for educational use)

## 13. Glossary

- **B-Tree**: A self-balancing tree data structure that maintains sorted data
- **Leaf Node**: A B-Tree node that stores actual key-value pairs
- **Internal Node**: A B-Tree node that stores keys and child pointers
- **Page**: A fixed-size (4KB) unit of storage on disk
- **Pager**: Component that manages page I/O operations
- **Split**: Operation that divides an overfull node into two nodes
- **Root Page**: The top-level node of the B-Tree
- **Magic Bytes**: File signature used to identify database files
- **REPL**: Read-Eval-Print Loop, an interactive command-line interface
- **Serialization**: Converting in-memory data structures to bytes
- **Deserialization**: Converting bytes back to in-memory data structures

## 14. References

- [B-Tree Data Structure](https://en.wikipedia.org/wiki/B-tree) - Wikipedia article on B-Trees
- [Database Internals](https://www.oreilly.com/library/view/database-internals/9781492040331/) - Book on database implementation
- [SQLite Architecture](https://www.sqlite.org/arch.html) - SQLite's page-based architecture
- [Rust Book](https://doc.rust-lang.org/book/) - Official Rust documentation

---

**Document Version**: 1.0  
**Last Updated**: 2024  
**Status**: Active  
**Owner**: BTreeDB Development Team

