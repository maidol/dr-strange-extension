# 插件：deps

[English](README.md) · 简体中文

将**构建清单**读成图中的事实：一个项目声明了它依赖什么，以及依赖哪个版本。
manifest 为 `deps@1`，**不声明任何文件扩展名**——见[分发](#分发)。纯 Rust：
`serde_json`、`toml_edit` 与 `quick-xml`，另加两种按行解析的格式。

每个代码插件在有东西 import 某个外部包的那一刻，就已经为它铸造了节点——ts 解析器
在文件写下 `import express` 时就会写出 `express`。但它们都无从得知：项目在某个它们
不读的文件里**声明**了对它的依赖，并且指定了版本。这两半只差一个键，却从不相遇，
于是"这个文件 import 的东西，版本是多少"在图中无解。

本插件读的正是那份声明，并且按代码插件为 import 定键的方式为依赖定键。于是两者
成为**同一个节点**。

## 分发

其他地方的路由都按扩展名，因为输入本就是文件类型。而清单是**文件名**：
`package.json` 不等于"所有 `.json`"，声明该扩展名会把树中每个 fixture 和
`tsconfig` 都从本该处理它们的读取器那里夺走。

因此本插件不声明扩展名，改由**宿主**把它认识的清单文件名路由给以 `deps` 之名安装的
插件——这与宿主把 `.git` 目录交给 `git` 是同一种声明。宿主保存的是这类插件的
*列表*，所以同时运行两套构建系统的仓库会被两者分别读取。

不作任何猜测：本插件缺席时，这些文件仍与从前一样按散文读取，报告也会说明哪些扩展名
无人认领。

| 文件 | 从中读取 | 依赖键 |
|---|---|---|
| `package.json` | `name`；`dependencies`、`devDependencies`、`peerDependencies`、`optionalDependencies` | 包名——正是 ts 解析器为裸说明符铸造的键 |
| `go.mod` | `module`；`require`，单行与块形式皆可 | 模块路径——正是 go 解析器为 import 铸造的键 |
| `requirements.txt` | 每行一条需求 | 发行包名 |
| `pyproject.toml` | PEP 621 的 `[project]`，以及 poetry 的表 | 发行包名 |
| `pom.xml` | 项目自身坐标；`<dependencies>` | `group:artifact` |
| `build.gradle`、`build.gradle.kts` | `implementation`／`api`／`testImplementation` 等行 | `group:artifact` |

## 节点与边

| | 含义 |
|---|---|
| `Manifest` | 每个文件一个，以路径为键，带 `declares`——本清单所属的包。monorepo 中的多个 `package.json` 就是多个节点 |
| `Package` + `External` | 依赖本身，按其生态的命名方式定键。`External` 是一项断言：该键指称树之外的东西；两个处理器断言同一个外部键，是彼此一致而非冲突——这正是本节点与 import 已铸造的节点成为**同一个节点**的原因 |
| `DEPENDS_ON` | 清单 → 依赖，边上带原样记录的 `version` 与 `scope`（`runtime`、`dev`、`test`、`optional`） |

版本记在**边**上而非包上：版本是关于*这一次声明*的事实，同一仓库中的两份清单完全
可能声明不同的版本。

## 这一联结在哪里成立，在哪里不成立

- **npm 与 go 干净地联结。** 被声明的 `express` 与被 import 的 `express` 是同一个键，
  因而是同一个节点；Go 模块路径亦然。
- **Python 靠运气联结。** 发行包名并不是 import 根——`requirements.txt` 写 `pillow`，
  而代码写 `PIL`。两者不同时，这条声明会落在没有任何 import 会命名的节点上。
- **Maven 与 Gradle 完全不联结。** 坐标是 `com.google.guava:guava`，而 java 解析器
  按包定键（`com.google.common.…`）。没有猜测就无法把两者对应起来，而这个家族不猜。
  依赖仍然被记录——构建声明了什么，即便无法据此回答"谁 import 了它"，也依然值得
  知道——只是它的替身节点独自存在。

## 已知局限

**gradle 构建是程序，不是清单。** 依赖可以被计算出来、通过 version catalog 取别名，
或由插件贡献。本插件只读寻常写法，别无其他：`implementation 'g:a:v'`、
`api("g:a:v")`。`implementation(libs.something)` 指向一个此处无法解析的 catalog
条目，因此跳过而不猜测。

**不读锁文件。** `package-lock.json`、`go.sum` 之流陈述的是*已解析*的图而非声明；
它们体量巨大、每次安装都变，且回答的是另一个问题。

**`Cargo.toml` 仍归 `toml` 插件**，它已把该文件读成表。`pyproject.toml` 则**不**：
该文件名路由到这里，因为依赖语义是通用 TOML 读取器无从知晓的。在本插件出现之前
摄取的平面里，该文件是 `Table` 节点，之后则是 `Manifest`。

**解析失败的清单仍会产出它的文件节点**，并被计数与点名。仓库里出现写了一半的清单
比谁都愿意承认的更频繁，为其中一个而拒绝整次摄取是荒唐的。

## 目录条目

发布工作流只会为**已列于** `catalog.json` 的插件写入 `version`、`url` 与
`sha256`，而拒绝为尚未列入者凭空造一行——`claims` 与 `min_drsg` 是判断，不是
机械操作。本插件尚未进入目录，因为在首次发布之前这两项都无法如实陈述：哈希取自
已发布的构件，而将清单文件名分发给 `deps` 的宿主尚未发布。请在打 `deps-v1.0.0`
标签时手工添加：

| 字段 | 取值 |
|---|---|
| `claims` | `build manifests`——此处扩展名一列是散文，与 `git` 相同，因为这两个插件都不经由扩展名抵达 |
| `min_drsg` | `2.7.0`——把清单文件名路由到本插件的那个版本。更旧的宿主永远不会把任何东西交给它 |

在该行出现之前，构件仍可按 URL 安装；只有需要经目录解析名字的
`drsg plugin install deps` 才依赖它。

## 构建与测试

```console
$ cd plugins/deps && cargo test    # 8 个原生测试
$ just deps-plugin                 # → target/wasm32-wasip2/release/drsg_plugin_deps.wasm
$ drsg plugin install …/drsg_plugin_deps.wasm
```
