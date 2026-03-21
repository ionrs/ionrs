use criterion::{criterion_group, criterion_main, Criterion};
use ion_core::engine::Engine;

fn bench_fibonacci(c: &mut Criterion) {
    let src = r#"
        fn fib(n) {
            if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
        }
        fib(20)
    "#;

    let mut group = c.benchmark_group("fibonacci");
    group.bench_function("tree_walk", |b| {
        b.iter(|| {
            let mut engine = Engine::new();
            engine.eval(src).unwrap()
        })
    });
    group.bench_function("vm", |b| {
        b.iter(|| {
            let mut engine = Engine::new();
            engine.vm_eval(src).unwrap()
        })
    });
    group.finish();
}

fn bench_loop_sum(c: &mut Criterion) {
    let src = r#"
        let mut sum = 0;
        for i in 0..1000 {
            sum += i;
        }
        sum
    "#;

    let mut group = c.benchmark_group("loop_sum_1000");
    group.bench_function("tree_walk", |b| {
        b.iter(|| {
            let mut engine = Engine::new();
            engine.eval(src).unwrap()
        })
    });
    group.bench_function("vm", |b| {
        b.iter(|| {
            let mut engine = Engine::new();
            engine.vm_eval(src).unwrap()
        })
    });
    group.finish();
}

fn bench_list_map_filter(c: &mut Criterion) {
    let src = r#"
        let items = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let result = items
            .map(|x| x * x)
            .filter(|x| x > 20);
        result
    "#;

    let mut group = c.benchmark_group("list_map_filter");
    group.bench_function("tree_walk", |b| {
        b.iter(|| {
            let mut engine = Engine::new();
            engine.eval(src).unwrap()
        })
    });
    group.bench_function("vm", |b| {
        b.iter(|| {
            let mut engine = Engine::new();
            engine.vm_eval(src).unwrap()
        })
    });
    group.finish();
}

fn bench_string_ops(c: &mut Criterion) {
    let src = r#"
        let mut result = "";
        for i in 0..100 {
            result = result + "x";
        }
        result.len()
    "#;

    let mut group = c.benchmark_group("string_concat_100");
    group.bench_function("tree_walk", |b| {
        b.iter(|| {
            let mut engine = Engine::new();
            engine.eval(src).unwrap()
        })
    });
    group.bench_function("vm", |b| {
        b.iter(|| {
            let mut engine = Engine::new();
            engine.vm_eval(src).unwrap()
        })
    });
    group.finish();
}

fn bench_match_heavy(c: &mut Criterion) {
    let src = r#"
        fn classify(n) {
            match n % 4 {
                0 => "zero",
                1 => "one",
                2 => "two",
                _ => "three",
            }
        }
        let mut count = 0;
        for i in 0..200 {
            if classify(i) == "zero" { count += 1; }
        }
        count
    "#;

    let mut group = c.benchmark_group("match_heavy_200");
    group.bench_function("tree_walk", |b| {
        b.iter(|| {
            let mut engine = Engine::new();
            engine.eval(src).unwrap()
        })
    });
    group.bench_function("vm", |b| {
        b.iter(|| {
            let mut engine = Engine::new();
            engine.vm_eval(src).unwrap()
        })
    });
    group.finish();
}

fn bench_comprehension(c: &mut Criterion) {
    let src = r#"
        [x * x for x in 0..50 if x % 2 == 0]
    "#;

    let mut group = c.benchmark_group("comprehension_50");
    group.bench_function("tree_walk", |b| {
        b.iter(|| {
            let mut engine = Engine::new();
            engine.eval(src).unwrap()
        })
    });
    group.bench_function("vm", |b| {
        b.iter(|| {
            let mut engine = Engine::new();
            engine.vm_eval(src).unwrap()
        })
    });
    group.finish();
}

fn bench_closure_chain(c: &mut Criterion) {
    let src = r#"
        fn apply(f, x) { f(x) }
        fn compose(f, g) { |x| f(g(x)) }
        let double = |x| x * 2;
        let inc = |x| x + 1;
        let f = compose(double, inc);
        let mut sum = 0;
        for i in 0..100 {
            sum += apply(f, i);
        }
        sum
    "#;

    let mut group = c.benchmark_group("closure_chain_100");
    group.bench_function("tree_walk", |b| {
        b.iter(|| {
            let mut engine = Engine::new();
            engine.eval(src).unwrap()
        })
    });
    group.bench_function("vm", |b| {
        b.iter(|| {
            let mut engine = Engine::new();
            engine.vm_eval(src).unwrap()
        })
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_fibonacci,
    bench_loop_sum,
    bench_list_map_filter,
    bench_string_ops,
    bench_match_heavy,
    bench_comprehension,
    bench_closure_chain,
);
criterion_main!(benches);
