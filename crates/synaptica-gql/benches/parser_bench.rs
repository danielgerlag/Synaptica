use criterion::{black_box, criterion_group, criterion_main, Criterion};
use synaptica_gql::lexer::Lexer;
use synaptica_gql::parser;

const SIMPLE_QUERY: &str = "MATCH (n:Person) RETURN n.name";

const COMPLEX_QUERY: &str = "\
    MATCH (p:Person)-[:KNOWS]->(f:Person) \
    WHERE p.age > 25 AND f.name != 'Bob' \
    ORDER BY p.name ASC \
    LIMIT 10";

const INSERT_QUERY: &str = "\
    INSERT (n:Person {name: 'Alice', age: 30})-[:KNOWS {since: 2020}]->(m:Person {name: 'Bob'})";

fn bench_lex_simple(c: &mut Criterion) {
    c.bench_function("lex_simple", |b| {
        b.iter(|| {
            Lexer::new(black_box(SIMPLE_QUERY)).tokenize().unwrap();
        });
    });
}

fn bench_lex_complex(c: &mut Criterion) {
    c.bench_function("lex_complex", |b| {
        b.iter(|| {
            Lexer::new(black_box(COMPLEX_QUERY)).tokenize().unwrap();
        });
    });
}

fn bench_parse_simple(c: &mut Criterion) {
    c.bench_function("parse_simple", |b| {
        b.iter(|| {
            parser::parse(black_box(SIMPLE_QUERY)).unwrap();
        });
    });
}

fn bench_parse_complex(c: &mut Criterion) {
    c.bench_function("parse_complex", |b| {
        b.iter(|| {
            parser::parse(black_box(COMPLEX_QUERY)).unwrap();
        });
    });
}

fn bench_parse_insert(c: &mut Criterion) {
    c.bench_function("parse_insert", |b| {
        b.iter(|| {
            parser::parse(black_box(INSERT_QUERY)).unwrap();
        });
    });
}

criterion_group!(
    benches,
    bench_lex_simple,
    bench_lex_complex,
    bench_parse_simple,
    bench_parse_complex,
    bench_parse_insert,
);
criterion_main!(benches);
