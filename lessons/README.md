# Rust Learning Curriculum: Building Real-World Applications

This curriculum teaches Rust concepts through the lens of building a real-world Terminal User Interface (TUI) application - the SomaFM CLI radio player. Each lesson builds on the previous one, using actual code from the project to illustrate concepts and architectural patterns.

## Course Overview

**Target Audience**: Experienced programmers new to Rust
**Focus**: Practical Rust concepts and real-world application architecture
**Approach**: Learn by examining and improving working code

## Lessons

### [Lesson 1: Project Structure & Module System](./01-project-structure-modules.md)
**Core Concepts**: Cargo.toml, module organization, visibility, dependency management
- How to organize Rust projects for maintainability
- Understanding Cargo's dependency and feature system
- Module system and import patterns
- **Exercises**: Add configuration support, improve error handling, reorganize modules

### [Lesson 2: Ownership, Borrowing & Async Patterns](./02-ownership-borrowing-async.md)
**Core Concepts**: Ownership, borrowing, `Arc<Mutex<T>>`, async/await
- How ownership works in practice with async code
- Shared state patterns with `Arc<Mutex<T>>`
- Avoiding borrow checker issues in async functions
- **Exercises**: Fix borrowing issues, implement caching, improve resource management

### [Lesson 3: Data Modeling & Serialization](./03-data-modeling-serialization.md)
**Core Concepts**: Serde, custom deserializers, error handling, API integration
- Designing robust data structures for external APIs
- Handling inconsistent data with custom deserializers
- Error handling strategies for data processing
- **Exercises**: Handle more API formats, add caching, implement validation

### [Lesson 4: Concurrent Programming & Message Passing](./04-concurrent-programming-message-passing.md)
**Core Concepts**: Channels, worker patterns, debouncing, non-blocking operations
- Building responsive UIs with background workers
- Message passing patterns with tokio channels
- Debouncing and rate limiting strategies
- **Exercises**: Add error display, request prioritization, cancellation, retry logic

### [Lesson 5: Real-World Application Architecture](./05-real-world-application-architecture.md)
**Core Concepts**: Layered architecture, resource management, performance, testing
- Designing maintainable application architectures
- Resource management and graceful shutdown
- Performance optimization and monitoring
- **Exercises**: Complete error handling, performance monitoring, configuration, plugins

## Key Learning Outcomes

After completing this curriculum, you'll understand:

1. **Project Organization**: How to structure real Rust applications with clear module boundaries
2. **Memory Safety**: Practical ownership and borrowing patterns that work with async code
3. **Type Safety**: Building robust data pipelines with serde and custom serialization
4. **Concurrency**: Message passing patterns for responsive, concurrent applications
5. **Architecture**: Designing maintainable, testable, production-ready applications

## Prerequisites

- Experience with programming (any language)
- Basic understanding of command-line tools
- Rust installed on your system (`rustup`, `cargo`)

## How to Use This Curriculum

1. **Read Each Lesson**: Start with the concepts, then examine the code examples
2. **Complete Exercises**: Each lesson has practical exercises that build on the examples
3. **Run the Code**: Clone the repository and experiment with the actual application
4. **Implement Improvements**: Many exercises involve improving or extending the existing code

## Running the SomaFM CLI

To run the application and see the concepts in action:

```bash
# Install and run
cargo run

# Or run without audio dependencies
cargo run --bin somafm-no-audio

# Enable debug logging to see internal operations
RUST_LOG=debug cargo run
```

## Project Structure

```
somafm-cli/
├── src/
│   ├── main.rs           # Entry point and event loop
│   ├── app.rs            # Application controller
│   ├── ui.rs             # Terminal UI rendering
│   ├── api.rs            # SomaFM API integration
│   ├── audio.rs          # Audio playback
│   ├── actions.rs        # Message types
│   └── bin/
│       └── somafm-no-audio.rs  # Alternative binary
├── lessons/              # This curriculum
│   ├── 01-project-structure-modules.md
│   ├── 02-ownership-borrowing-async.md
│   ├── 03-data-modeling-serialization.md
│   ├── 04-concurrent-programming-message-passing.md
│   ├── 05-real-world-application-architecture.md
│   └── README.md
└── Cargo.toml           # Project configuration
```

## Additional Resources

- [The Rust Programming Language Book](https://doc.rust-lang.org/book/)
- [Async Programming in Rust](https://rust-lang.github.io/async-book/)
- [Tokio Tutorial](https://tokio.rs/tokio/tutorial)
- [Serde Documentation](https://serde.rs/)
- [Ratatui Documentation](https://ratatui.rs/)

## Contributing

Found an issue or have suggestions for improving the lessons? Feel free to open an issue or submit a pull request. The goal is to make this curriculum as helpful as possible for learning practical Rust development.

---

**Happy Learning!** 🦀

The best way to learn Rust is to build real applications. This curriculum gives you a foundation in the patterns and concepts you'll use every day as a Rust developer.