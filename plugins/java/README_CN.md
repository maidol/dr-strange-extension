# 插件：java

[English](README.md) · 简体中文

将 Java 解析为图事实。Manifest 为 `java@2`，处理 `.java`。基于
[tree-sitter-java](https://github.com/tree-sitter/tree-sitter-java)——不存在
成熟的纯 Rust Java 前端，而每个编辑器都已信任的语法比一个还需赢得信任的
语法是更好的地基。C 运行时与语法经 wasi-sdk 的 clang 编译到
`wasm32-wasip2`。

## 目录结构

```
parser/     drsg-java-parser——语言逻辑，25 个原生测试
component/  drsg-plugin-java——Guest 实现 + rmp-serde 部分结果（构建需要 wasi-sdk）
```

## 键

Java 自己的全限定名。**文件不是节点**：在 Java 里类型是单元，包是容器。

```
com.acme.core                         包
com.acme.core.Engine                  类型
com.acme.core.Engine.start            成员
com.acme.core.Engine.Builder          嵌套类型经由外层串接
```

## 节点

| 标签 | 产生于 | `doc_comment`（javadoc）/ `visibility` / `file` / `line` / `end_line` 之外的属性 |
|---|---|---|
| `Package` | 每个 `package` 声明一个 | `name`；`package-info.java` 的 javadoc。包在两端都被解析时经 `CONTAINS` 嵌套 |
| `Class` / `Interface` / `Enum` / `Record` / `Annotation` | 类型声明 | `fields`：described 的 `name: type` 列表（record 的来自其头部）；枚举常量入 `variants` |
| `Method` | 方法与构造函数——重载共享键：一个节点，先见者（带文档）胜 | `signature`（`void connect(int timeout)`） |
| 替身 | 外部类型/成员 | `Class` / `Interface` / `Annotation` / `Function`（引用证明了什么）+ `External` |

每个声明还带有 `end_line`：它在何处结束。于是 `snippet` 读取的正是这个声明本身，
而不是从首行起固定的若干行；读者也无需打开文件就能看出一样东西是六行还是两百行。
`line` 仍然指向**名字**所在处，因此声明上方的文档不会挪动图中记录的起始位置。

## 边

| 类型 | 含义 | `line` |
|---|---|---|
| `CONTAINS` | 包 → 类型 → 成员、外层类型 → 嵌套类型 | 声明处 |
| `HAS_METHOD` | 接口/注解 → 其要求的方法 | 成员 |
| `CALLS` | 方法 → 被调者；`new Foo(…)` 是对类型的调用；`super.m()` 沿 extends 链行走 | 调用处 |
| `IMPORTS` | 文件的每个顶层类型 → 文件导入的对象（Java 的 import 以文件为作用域；读者靠类型导航） | import 语句 |
| `EXTENDS` / `IMPLEMENTS` | 类 → 父类 / 接口；接口 → 被扩展接口；泛型基类延伸到它所下标的对象（`ArrayList<Double>` → `java.util.ArrayList`） | 类型声明 |
| `ANNOTATED_BY` | 类型/方法 → 其注解——Spring 代码库里注解*就是*架构（`@Service`、`@Transactional`、`@GetMapping`）；`java.lang` 自带的记号（`@Override`、`@Deprecated`……）是噪音，不入图 | 注解处 |
| `USES_TYPE` | 声明 → 为其**定型**的类型；`role` 记在边上（`field`、`param`、`return`），record 的组件计为字段。泛型参数会被走入——`List<Job>` 就是对 `Job` 的依赖——且只有本树声明的类型才成边 | — |

## 解析——按 javac 的方式读引用

顺序：**照原样已限定** → **同包**（无需 import——这是语言自己的规则）→
**单类型 import** → **通配 import**，对树内实际持有的类型解析 →
**`java.lang`**（无需 import 即知：`String`、`System`、常见异常）。

- **大写开头的接收者**（`Helper.create()`、`com.acme.Helper.create()`）
  是写下来的类型引用；**小写开头的接收者**是值——编译器的事，计数。
- **继承调用**——无接收者的 `helper()` 与 `super.helper()`——沿**树内
  extends 链**行走，找到声明该方法的类型。
- **静态 import** 直接绑定方法名。
- 上述途径均未命中的引用被计数，绝不猜测。

报告注记：值接收者的未解析调用 · 外部调用 · 合并的声明。

## 测试代码

"谁调用了它"这个问题，测试代码与生产代码给出的答案分量不同；分不清两者的读者
会把一个测试算作一个使用者。两条属性说明此事：`test_flag` 记录**依据是什么**，
`_test_flag_confidence` 记录**该依据有多少分量**——下划线前缀使后者不进入紧凑
渲染，也不进入向量，那里它只会是噪声；`cypher`、`get_node` 与 `export` 仍会返
回它，而衡量此标记的读者正是在那里查看。源码中没有写下依据时，不作任何标记。

| `test_flag` | `_test_flag_confidence` | 依据 |
|---|---|---|
| `annotation` | `definitive` | JUnit/TestNG 注解——`@Test`、`@ParameterizedTest`、`@RepeatedTest`、`@TestFactory`、`@TestTemplate`，以及 `@Before*`/`@After*` 生命周期钩子 |
| `build-layout` | `strong` | 位于 `src/test/java` 之下 |

两条规则分级，留下的是更强的那条：源码集是 Maven 与 Gradle 共同强制的布局，但
构建文件仍可改写；而注解是 JUnit 自己对测试的定义。它同时覆盖方法与持有该方法
的类型——类中含有 `@Test` 即为测试类——因此位于 `src/main/java` 的带注解测试
照样被标记，而 `src/test/java` 中未带注解的辅助方法保留较弱的标记。

## 选项（`[plugins.java]`）

| 键 | 效果 |
|---|---|
| `include_source = "true"` | 为声明附加 `_code` |

## 构建与测试

```console
$ cd parser && cargo test             # 25 个测试——原生运行，C 语法同样为宿主编译
$ just java-plugin                    # 需要 wasi-sdk；WASI_SDK 环境变量可覆盖默认路径
$ drsg plugin install component/target/wasm32-wasip2/release/drsg_plugin_java.wasm
```

## 已知局限

重载按名字归并（所有重载共享一个节点）；extends 链止步于树外类型；lambda
体内的调用归于外层方法（读者查看之处）；`var` 局部变量是值，经其调用被
计数。
