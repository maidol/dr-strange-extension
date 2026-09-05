# 插件：rust

[English](README.md) · 简体中文

将 Rust 源码解析为图事实。Manifest 为 `rust@2`，处理 `.rs`。基于
[syn](https://crates.io/crates/syn)——宏生态自身运行其上的解析器——只做
解析：不做类型推断、不展开宏，而这正是重点。`@2` 是**事实格式版本**
（事实的形状从库内原型起变更过一次），与 release 标签相互独立。

## 目录结构

```
parser/     drsg-rust-parser——语言逻辑，普通库，71 个原生测试
component/  drsg-plugin-rust——wasm 封装：Guest 实现 + rmp-serde 部分结果
```

## 键

条目的身份是它的**模块路径**——Rust 程序员如何称呼它，模型就如何认得它：

```
my_crate                              lib.rs（命名 crate 根，而非 "lib"）
my_crate::api::cache                  模块（文件或内联）
my_crate::api::cache::brute_force_search
my_crate::Thing::read                 固有方法
<my_crate::Thing as core::fmt::Display>::fmt    trait 实现方法——真实的
                                      限定路径语法，也是让六个 `impl From<…>`
                                      块不至于都抢占同一个键的唯一方法
```

crate 名来自最近的 `Cargo.toml` 的 `[package] name`（`-` → `_`），经宿主
读取——因此以 `…/foo/src` 为根的摄取仍然键为 `foo::…`，两个 crate 的
`api::Thing` 永不合并。

## 节点

| 标签 | 产生于 | `doc_comment` / `visibility` / `file` / `line` 之外的属性 |
|---|---|---|
| `Module` | 每个文件与每个内联 `mod` | `path`（相对 crate 根：`src/compute/cache.rs`）、`imports`（解析后的 use 目标，连接为串） |
| `Function` / `Method` | 自由函数、impl 函数（带 `self` 才是 `Method`） | `signature`、`returns`、`receiver`、`local_bindings`、`is_async`（仅为真时出现） |
| `Struct` / `Enum` / `Union` | 类型声明 | `fields`（described 列表，`vis name: type`，声明顺序）/ `variants`（described 列表，`Unit`、`Lit(i64)`、`A = 1`）；标注时有 `non_exhaustive` |
| `Trait` | trait 声明 | 其成员成为节点，经 `HAS_METHOD` 到达 |
| `Const` / `Static` | 常量与静态量 | 类型在 `signature`，初始化式在 `value`——**照原样记录，绝不求值**（`256 * 1024` 保持为表达式） |
| `TypeAlias` | `type X = …` | 被别名的类型在 `signature` |
| `Macro` | `macro_rules!` 定义 | — |
| 替身（stand-in） | 被引用但未在此声明的一切 | 标签表明引用证明了什么（`Function`、`Trait`、`Type`，仅见 `use` 时为裸 `External`）+ 额外标签 `External`；**无属性**——对替身而言键即事实 |

配置 `include_source = "true"`（来自 `[plugins.rust]`）时，每个条目附带
`_code`：照原样的源码，described 为仅供检索——`_` 前缀使其不进入 embedding
与模式摘要。

## 边

| 类型 | 含义 | `line` |
|---|---|---|
| `CONTAINS` | 模块 → 条目、类型 → 变体 | 声明处 |
| `HAS_METHOD` | trait/类型 → 其方法 | 方法所在行 |
| `CALLS` | 函数 → 它调用的对象 | **调用处** |
| `IMPLEMENTS` | 类型 → trait（`impl` 块）；`From<i64>` 作为边上的 `impl` 属性存在，而非另铸一个 `From` 节点 | `impl` 关键字 |
| `IMPORTS` | 模块 → 其 `use` 语句命名的对象（有别名时带 `as_written`） | `use` 语句 |
| `INSTANTIATES` | 函数 → 它**构造**的类型；构造的是枚举的变体时，`variant` 记在边上。构造不是调用：`Ok(v)`、`Mine::A(v)`、`Meters(1.0)`、`Widget { .. }` 都写得像调用或字面量，且都命名一个*类型*，因此不会为并非条目的构造器铸造 `Function` 节点 | 构造处 |
| `INVOKES` | 模块 → 条目位置的宏调用，`arguments` described 在边上——一个**被标记的盲区**：没有任何东西展开宏，其定义的条目缺席，但定义发生之处可寻 | 调用处 |
| `REFERENCES` | 函数 → 它**作为值传递**（而非调用）的函数 | 该实参 |
| `USES_TYPE` | 声明 → 为其**定型**的类型；`role` 记在边上，说明是哪个位置（`field`、`param`、`return`、`variant`、`alias`），同一对写了多次则取并集。泛型参数会被走入——`mpsc::Sender<Job>` 就是对 `Job` 的依赖——且只有本树声明的类型才成边，外来类型仍以文本留在 `signature`/`fields` 中。绝不自环 | — |

## 解析——确定性的界线

- 写成**路径**的调用（`fs::read(…)`、`Vec::new()`）按文件自身的 `use`
  列表展开并精确绑定——以每条 `use` **引入的**名字为准，因此
  `use a::b as c` 让 `c` 而非 `b` 进入作用域；此处无人声明的路径成为外部替身（"这个 crate 用了
  那个东西"需要的正是这个）。
- **裸名字**按作用域就近绑定；有两个等距候选的名字**视为歧义——计数，
  不猜测**。
- **方法调用**（`.read()`）不写路径；它能解析到什么程度，取决于接收者的
  类型能否**从函数体里读出来**。类型参数贯穿函数体写下的每一种绑定：参数或
  `let` 的类型标注、字段的声明类型、构造路径（`Vec::new()`）、声明的返回
  类型、填入参数后的 `type` 别名、常量，以及它们的链式组合
  （`self.items.iter().map(…)`）——经由一张 std 自身方法返回什么的表——
  于是 `for n in &v`、`if let Some(n) = …`、对本树枚举的 `match` 分支、
  `let (a, b) = …`、`Node { key, .. }` 和 `v.iter().map(|n| …)` 的闭包参数
  都有类型，`?`/`.unwrap()` 抵达 `Result<T>` 的 `T`，函数体是链式表达式的
  闭包说明 `map` 产出什么，`collect::<Vec<_>>()` 是 `Vec`。**通道构造器**即它返回的那一对——
  `let (tx, rx) = mpsc::channel()` 为两半都定了类型，于是 `tx.send(v)` 与
  `rx.recv()` 落到 `mpsc::Sender::send` 与 `mpsc::Receiver::recv`，而非账本；
  键取自构造器自身的模块（`std::sync::mpsc`、`tokio::sync::mpsc`、
  `crossbeam::channel`），因此与带注解的 `let rx: mpsc::Receiver<T>` 解析到
  同一个节点，不会为同一类型另铸一种拼写。本树声明的类型
  落到它自己的方法上；`T: Tr`、`impl Tr` 与 `dyn Tr` 落到 trait 的方法上；
  本树未声明的类型——std 的、依赖的——落到外部替身上，其键取**声明**该方法
  的类型：`Vec::push`、`str::trim`、`Clone::clone`，以及每个通过 trait 作答
  的具体迭代器（`Range`、`Lines`、`Chars`）共用的 `Iterator::collect`——这就
  是关于它所知的全部。没有任何东西陈述其类型的接收者——无约束的泛型、没有任何表
  认识的外部函数的结果——**计数，绝不猜测**，且账本边会说明是哪一跳停下的。
- **重导出**（`pub use`，含 `pub(crate) use`）创建后续引用赖以解析的
  门面路径。
- 同一键出现两次几乎总是同一条目的两个 `#[cfg]` 分支——在此解决（先见者
  胜）并计数，不当作冲突。

每项计数都落入报告注记，让稀疏的图自己解释自己：未解析的方法调用、外部
调用、歧义名字、未展开的宏调用。

## 测试代码

"谁调用了它"这个问题，测试代码与生产代码给出的答案分量不同；分不清两者的读者
会把一个测试算作一个使用者。两条属性说明此事：`test_flag` 记录**依据是什么**，
`_test_flag_confidence` 记录**该依据有多少分量**——下划线前缀使后者不进入紧凑
渲染，也不进入向量，那里它只会是噪声；`cypher`、`get_node` 与 `export` 仍会返
回它，而衡量此标记的读者正是在那里查看。源码中没有写下依据时，不作任何标记。

| `test_flag` | `_test_flag_confidence` | 依据 |
|---|---|---|
| `attribute` | `definitive` | `#[test]` 一族属性——裸写的那个、运行时自己的（`#[tokio::test]`、`#[async_std::test]`）、`#[bench]`、`#[rstest]` |
| `build-rule` | `definitive` | 位于 `#[cfg(test)]` 模块内，或位于 `tests/`、`benches/` 下的文件 |

两者都是 cargo 的规则而非约定：`#[cfg(test)]` 模块会被编译出库之外，`tests/`
与 `benches/` 是各自独立的目标，从不链接进库。两者都是**作用域**，因此也覆盖写
在其中的 `impl` 块——这些块的方法要到 assemble 阶段才定键。先写者胜出，且更窄
的规则先跑：`#[cfg(test)]` 模块内的 `#[test]` 函数保留 `attribute`。cfg 按 token
读取而非按文本，这同时化解了两个陷阱——`cfg(feature = "test")` 携带的是字符串
而非标识符，`cfg(not(test))` 的分组会被跳过。至于 `#[cfg(test)] mod tests;` 其
主体在另一个**文件**中的情形，只有当该文件本身位于已标记的作用域内才会被标记：
它由哪个模块声明，要到 assemble 才知道。

## 选项（drsg.toml 的 `[plugins.rust]`）

| 键 | 效果 |
|---|---|
| `include_source = "true"` | 为条目附加 `_code` |

## 构建与测试

```console
$ cd parser    && cargo test          # 71 个测试，无需 wasm 工具链
$ just rust-plugin                    # → component/target/wasm32-wasip2/release/drsg_plugin_rust.wasm
$ drsg plugin install …/drsg_plugin_rust.wasm
```

部分结果以 **MessagePack**（`rmp-serde`）跨越阶段边界：选二进制因为大树的
部分结果以兆计，选自描述因为事实携带 `serde_json::Value` 属性——部分结果
的格式是插件自己的事，宿主从不查看。

## 已知局限

宏生成的条目缺席（由 `INVOKES` 标记）；泛型接收者上的 trait 方法调用属于
方法调用，因而被计数；链式调用经过一个解析器没有其返回类型事实的 std 方法
（`map.get(k)?.m()`）便到此为止；`#[cfg]` 的取舍不做求值——两个分支的条目
都存在，重复者计数。
