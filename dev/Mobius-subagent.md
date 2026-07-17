# Mobius Subagent Skill 蓝图

## 1. 目的与边界

Mobius Subagent 是一套供 main Agent 使用的委托工作流。它帮助 main Agent 选择适合的工作角色、构造任务、
调用 Codex 当前公开的原生能力，并消费执行结果、证据位置、副作用声明和不确定性。

系统只有两层：

1. **Codex Runtime 层**拥有 agent/thread、turn、item、权限、工具、模型选择和 token usage 等运行时事实；
2. **委托语义层**拥有角色、任务目的、副作用边界、结果结构、冻结输入声明和消费条件。

Mobius Subagent 不复制、封装或另建 Runtime 的线程历史、调用状态、权限、工具协议和用量数据。main Agent
直接消费当前宿主公开的原生对象和事件，并只补充 Runtime 不负责的委托语义。

本蓝图不定义下游业务状态、事实类型、验收状态机或持久化模型，也不要求 subagent 理解它们。main Agent 是
委托的唯一编排者和结果解释者；subagent 只执行收到的有界任务，不调用下游业务状态转移接口。main Agent
是否以及如何把返回结果转义为另一系统的对象，完全由该系统自己的接纳契约决定。

subagent 可以在任务授权和 Runtime 权限范围内改变 main Agent 之外的环境。`driver` 尤其可以修改工作区、
运行命令或执行其他明确副作用。它必须声明实际做了什么、影响了什么、如何核查以及还存在哪些未知；main
Agent 消费并理解这些影响。外部变化本身是世界事实，但 result envelope 只是候选 observation、effect、
artifact、advice 和 provenance；它不会自动取得任何下游事实地位。

`judge` 始终是 advisory subagent。它返回发现、反证、风险和建议处置；main Agent 独立决定如何解释和使用
这些意见。

本蓝图采用协作式 Agent 信任边界。main Agent 与 subagent 都被视为会遵守当前 Skill、任务 envelope 和角色
边界的协作者；角色边界是 instruction contract，不是针对恶意或被攻陷 Agent 的安全隔离。Runtime 可以向
subagent 暴露与 main Agent 相同的工具，但 subagent 仍不得调用下游业务 mutation 接口，也不得把自己提升为
下游事实接纳者或正式裁决者。本 skill 不要求 per-thread tool hiding、caller attestation 或角色专用 sandbox。

## 2. 设计不变量

1. **Main owns interpretation**：main Agent 独占委托编排、任务关联、结果解释和下游提交。
2. **One task, one result**：一次委托完成一个有界任务，并返回一份供 main Agent 消费的最终结果。
3. **No business continuity in workers**：subagent 不持有跨任务复用的业务状态；补充或重试默认创建
   新任务。
4. **Native runtime is authoritative**：线程、消息、工具调用、权限、状态、模型和用量以当前官方 Runtime
   对象为准。
5. **Effects are declared**：获得副作用权限的 subagent 必须声明实际、失败、部分完成、回滚、意外和待清理
   影响。
6. **Results are candidate inputs**：subagent 输出是 main Agent 的候选 observation、effect、artifact、
   advice 和 provenance，不能自行取得下游事实或决策地位。
7. **Freshness before consumption**：main Agent 在消费结果时检查任务依据的 baseline 与相关材料版本是否仍然有效。
8. **协作式 Agent 信任边界**：Runtime 决定实际可用的 sandbox、approval 和工具；Skill、任务 envelope 与
   模型遵循共同决定 subagent 在这些能力中实际使用什么。本工作流不要求按角色机械隐藏工具或认证 caller。
9. **Bounded return**：每个任务声明有限 result budget；final output 只保留结论、核查入口与必要结构，
   大型、重复或原始内容通过 artifact locator 返回，不能把 Runtime item 或材料全文复制进 main Context。

这里的“无状态”指 subagent 不拥有跨任务的业务状态。Driver 是当前 main thread 通过 host 原生 Subagent
workflow 创建的 child agent thread；Codex Runtime 负责它的执行上下文和线程状态，本 skill 不定义另一套
上下文复制机制。

## 3. 角色与模型选择

### 3.1 角色是工作模式

角色帮助 main Agent 选择默认工作方式，不产生新的业务实体，也不绑定任何下游生命周期：

| Role         | 工作方式                                             | 默认副作用边界                       |
| ------------ | ---------------------------------------------------- | ------------------------------------ |
| `scout`      | 调查本地代码、文件、日志、测试和数据                 | 只读                                 |
| `researcher` | 调查官方文档、论文、标准和开放世界事实               | 只读工作区；允许被授权的网络读取     |
| `driver`     | 延续 main Agent 当前工作，执行一个有界任务并声明影响 | 仅限任务明确授权的副作用             |
| `verifier`   | 独立复现、测试、观察和比对                           | 默认只读；允许声明过的临时测试副作用 |
| `judge`      | 审计冻结材料、寻找反证并提出 advisory disposition    | 只读                                 |

五个名称是可复用的工作 profile。角色身份由工作功能决定，不能由实际模型反推。

### 3.2 模型选择矩阵

以下矩阵是 main Agent 的推荐 execution policy，不是 availability 保证：

| Role             | 推荐能力与 effort                                             | 选择理由                               |
| ---------------- | ------------------------------------------------------------- | -------------------------------------- |
| `scout`          | 当前 host 可用的快速 coding model / `medium`                  | 清晰、可重复的本地调查与结构化摘要     |
| `researcher`     | 当前 host 可用的 research-capable model / `medium`            | 边界明确的开放世界检索、来源比较与综合 |
| `driver`         | 不固定 Driver 专属 model / effort；由 Runtime 解析            | 使用 host 原生 Subagent workflow       |
| `verifier`       | 当前 host 可用的可靠 coding model / `high`                    | 对明确 claims 和 checks 做可重复验证   |
| `judge` internal | 当前 host 可用的 strong-reasoning model / `medium`            | 复杂、开放式、反证优先的语义判断       |
| `judge` external | 一个已可用的独立外部模型 / 其支持的 effort                    | 在相同 Judge 语义下提供不同模型族视角 |

该矩阵按能力画像表达，是 execution policy，而不是 availability 保证或固定 model catalog。main Agent 在调用前
根据当前 host 支持的模型解析能力画像，确认显式请求的 model 和 effort 是否可用；不可用时选择最接近该工作负载的
当前可用配置。Driver 不固定专属 model 或 effort；实际选择由 Codex Runtime 解析。

输入很小、检查机械且反例空间有限时，可以降低 effort；研究、验证或审查变得复杂、开放、跨域或高影响
时，应升级到更强的当前可用模型或 effort。偏离矩阵不改变角色语义。

### 3.3 Driver 的唯一执行路径：host 原生 Subagent workflow

Driver 始终通过当前 host（Codex App、CLI 或 IDE）正式支持的原生 Subagent workflow 执行：

1. main Agent 请求 Codex 创建一个原生 subagent，并把 Driver 的有界任务语义写入完整 envelope；
2. Driver 不设置专属 model、reasoning effort、provider、sandbox 或 approval 覆盖；
3. Codex Runtime 负责 spawn、agent thread、follow-up、wait、interrupt 和 close；支持的客户端可以展示和检视线程；
4. Driver 在原生 agent thread 中执行授权动作并返回结果和 effect 声明；
5. main Agent 消费结果并独占解释、核查和任何下游提交。

Driver 是委托语义角色，不是需要 attestation 的 Runtime principal 或固定 agent type。本 skill 不要求 main
Agent 证明实际 profile 是某个未被覆盖的内建 agent；Runtime 负责解析实际 profile。观察到 spawn、配置、
Runtime 或权限错误时如实失败，不切换到自建 agent runtime、第二 transport 或后台 worker。

官方 Runtime 使 subagent 继承当前 sandbox policy 和 permission mode。本 skill 不推测其内部模型选择机制，
也不建立 Driver 专用 Runtime adapter。继承可能使 Driver 看见 main Agent 可见的同一组工具；Driver 仍按本
Skill、任务 envelope 与协作式 Agent 信任边界执行，不调用被 downstream Composition 明确禁止的接口。该
约束依赖 Skill 与模型遵循，而不是线程级 capability 隔离。

Driver 是 main Agent 当前任务的 child agent，但公共 Subagents 契约没有承诺完整父对话逐项复制。main
Agent 必须在任务中明确写入会改变执行正确性的当前决策、约束、目标和授权；其余线程上下文由 Codex
Runtime 按当前 host 的原生行为管理。

官方依据：[Codex Subagents](https://learn.chatgpt.com/docs/agent-configuration/subagents)。

## 4. 委托语义

### 4.1 每次启动都使用完整 basic envelope

main Agent 每次启动 subagent 都提供同一套 basic envelope。任务可以使用自然语言、结构化 Markdown 或
JSON；下面定义的是必须表达的语义，不是新的 Runtime transport schema：

```json
{
  "role": "scout | researcher | driver | verifier | judge",
  "background": {
    "why_now": "为什么在当前时点委托这项工作",
    "current_state": ["与本任务相关的现状和已完成工作"],
    "confirmed_facts": [
      {"id": "BF1", "fact": "main Agent 已确认的事实", "evidence": []}
    ],
    "materials": [
      {"id": "BM1", "locator": "可访问材料", "purpose": "为什么提供它"}
    ],
    "assumptions_to_check": [
      {"id": "BA1", "assumption": "不能直接当作事实的前提"}
    ]
  },
  "objectives": [
    {
      "id": "O1",
      "objective": "本次委托要得到的一个可观察结果",
      "priority": "must | should"
    }
  ],
  "boundaries": {
    "forbidden": [
      {"id": "F1", "rule": "禁止的动作、对象或越界行为", "reason": "原因"}
    ],
    "focus": [
      {"id": "FO1", "target": "主要对象、入口或关注范围", "purpose": "为什么从这里开始"}
    ]
  },
  "role_input": {},
  "output_format": {
    "representation": "structured_markdown | json",
    "template": "内联第 5 节公共输出和第 6 节所选角色的 role_output",
    "constraints": ["证据粒度、篇幅、locator 和脱敏要求"],
    "result_budget": {
      "max_public_result_bytes": 8192
    }
  },
  "done_when": [
    {
      "id": "D1",
      "condition": "成功、部分完成或明确阻塞时可观察的返回条件",
      "evidence_required": []
    }
  ]
}
```

- `background` 让 subagent 理解任务缘由和当前状态。`confirmed_facts` 只放 main Agent 已核查的事实；
  引用入口进入 `materials`；待验证前提进入 `assumptions_to_check`。
- `objectives` 必填且非空，可以包含多个独立、可逐项判断的目标。目标保持结论开放，并落在同一个连贯
  委托范围内；不应把彼此无关的工作强塞给同一个 subagent。
- `boundaries` 采用 forbidden-first。`forbidden` 必须显式出现，可以为空；`focus` 只给出主要对象、入口
  和注意力范围，不是穷举式目录或文件 allowlist。subagent 可以为实现 objectives 继续追踪相关依赖、调用
  方、测试、配置和证据，但必须遵守 `forbidden`、用户授权与 Runtime 权限。
- 当 downstream Composition 具有更窄的 ownership 边界时，main Agent 必须把禁止访问的全部 API、状态目录
  或提交面逐项加入 `forbidden`。这些 integration-specific 名称留在任务 envelope，不进入通用 Skill schema。
- 默认不写 `allowed`。缺少 `allowed` 不表示只读，也不限制 subagent 只能访问 `focus`、roots 或已列目录；
  角色语义、objectives 和 role input 共同表达普通任务可以采取的动作。只有存在重大安全、隐私、合规、
  生产环境、外部人员影响或不可逆风险时，才在 `boundaries` 中追加窄化的 `allowed` positive allowlist：

  ```json
  {
    "allowed": [
      {
        "id": "AL1",
        "action": "风险场景下唯一允许的动作",
        "target": "允许作用的对象",
        "constraints": ["严格限制"]
      }
    ]
  }
  ```

  一旦提供 `allowed`，它对相应高风险动作具有穷举约束；未匹配的动作不得主动执行。普通 Driver 修改仍由
  objectives、`change_targets`、forbidden 和 Runtime 权限约束，不需要为了授权而制造 `allowed` 清单。
- `role_input` 使用第 6 节对应角色的专用输入模板。
- `output_format` 必填。main Agent 将第 5 节公共输出与所选角色的 `role_output` 完整组合并内联给
  subagent；任务允许副作用时一并内联第 7 节 effect 格式。不能只发送“见第 6 节”这类对受托者不可见
  的引用。`result_budget.max_public_result_bytes` 是一个有限正整数，约束包括 `role_output`、inventory 和
  locator 在内的完整序列化公共结果；单个 item 也不能绕过该总上限。main Agent 按任务风险选择预算，而
  不是要求 subagent 返回材料全文。预算不足以承载原始或重复内容时，subagent 将内容写入已授权的稳定
  artifact，final output 只返回去重摘要、必要结论、可核查 locator 与明确 overflow，不能静默截断
  correctness-critical 结果。
- `done_when` 必填且非空，逐项说明何时返回，而不是把“完成”留给 subagent 自行定义。它既覆盖成功，
  也覆盖允许的部分完成和明确阻塞终止。

每个 correctness-critical 背景、目标、边界、输出要求和 DONE 条件都必须在 envelope 中自包含。Driver
可以利用 Codex Runtime 的原生线程上下文减少重复，但正确性不能依赖未公开或未承诺的父历史复制。下游业务
内部状态、持久化内容和与工作无关的 Runtime 元数据不进入任务；main Agent 应把影响执行正确性的业务事实
转义为普通背景、目标、边界或材料，而不是暴露另一系统的内部对象。

### 4.2 Main Agent 如何关联结果

main Agent 保留 Runtime 返回的原生 agent/thread 标识、任务 envelope identity 与本次 delegation baseline
的关联，用它等待、关闭、检查结果新鲜度和解释副作用。baseline 由 main Agent 选择，只需固定会改变结果
适用性的事实或材料版本；它不要求任何下游系统采用相同的 baseline 类型。此关联是一次委托的临时编排
信息，不进入下游业务状态，也不需要新 registry、queue、heartbeat 或 memory。

## 5. 返回结果语义

Runtime 的最终输出和 thread items 是传输事实。每个 subagent 返回一个公共 result envelope，并把角色
专用结果放入唯一的 `role_output`。不得把角色字段提升到根级、与公共字段合并或另交一份平行报告：

```json
{
  "status": "completed | partial | blocked | failed",
  "summary": "实际完成和发现",
  "objective_results": [
    {
      "objective_id": "O1",
      "status": "achieved | partial | blocked | failed",
      "result": "该目标实际得到的结果",
      "evidence": []
    }
  ],
  "assumption_results": [
    {
      "assumption_id": "BA1",
      "assessment": "confirmed | contradicted | inconclusive | not_evaluated",
      "impact": "它对结果的影响",
      "evidence": []
    }
  ],
  "done_when_results": [
    {
      "done_when_id": "D1",
      "status": "satisfied | unsatisfied | unknown | not_evaluated",
      "evidence": [],
      "reason": "必要时说明"
    }
  ],
  "boundary_compliance": {
    "status": "compliant | violation | unknown",
    "violations": [
      {
        "rule_ref": "F1、AL1，或发生无关任务漂移的 O1",
        "description": "实际或疑似越界",
        "effect_ids": [],
        "evidence": []
      }
    ]
  },
  "effects": [],
  "artifacts": [
    {"id": "A1", "locator": "可访问位置", "description": "产物说明"}
  ],
  "uncertainties": [
    {"subject": "仍不确定的事项", "reason": "原因", "next_check": "可选的下一步"}
  ],
  "blockers": [
    {"subject": "阻塞事项", "reason": "原因", "needed": "解除阻塞所需条件"}
  ],
  "overflow": {
    "omitted_items": 0,
    "artifact_ids": [],
    "reason": "none | result_budget"
  },
  "role_output": {}
}
```

公共 envelope 与角色输出的组合规则只有一条：

1. 根级字段表达所有角色共有的执行状态、目标与假设闭合、DONE 判断、边界遵守、effect、artifact、
   不确定项和 blocker；
2. `role_output` 严格使用第 6 节所选角色的一个专用输出模板；
3. `effects` 是唯一权威 effect inventory；角色输出只能用 `effect_ids` 引用，不能复制另一份 changes；
4. `artifacts` 是唯一权威 artifact inventory；角色输出使用 `artifact_ids` 引用；
5. 每个 `objective_id`、`assumption_id` 和 `done_when_id` 都逐项返回结果，即使结果是 unknown、未满足或
   未评估；
6. `boundary_compliance` 声明实际或疑似违反 forbidden、高风险 allowed 或偏离 objectives 的无关任务漂移；
   沿相关依赖开展探索不构成越界。该声明不替代 main Agent 对工具记录、diff 和外部对象的核查；
7. `status` 表达本次执行是否正常返回，不等同于所有 objectives 已达成，更不等同于任何下游接纳；
8. 事实性主张给出文件、行号、命令结果、URL、官方对象或其他可核查 locator；推断与观察分开；
9. 大型内容返回 locator 和摘要，避免复制 Runtime 已经保存的 item；
10. 公共字段与 `role_output` 共同遵守同一 whole-result byte budget，相同事实或 locator 只出现一次；超过预算的
    可选细节记录在 `overflow`，correctness-critical 结论若无法在预算内表达则返回 `partial` 或 `blocked`，
    不用 success-shaped 截断伪装完成。

这些字段仍是消费检查清单，不是需要另行版本化的传输协议。所有 ID 只在本次任务内使用，帮助 main Agent
检查每个输入是否闭合；`objectives`、`evidence`、`status`、`blocked` 等名称也只有本次委托内的普通含义，
不声明与任何下游同名对象或状态存在映射。该 envelope 不建立跨任务 registry、引用解析服务或新的业务
身份系统。

main Agent 消费结果时固定本次 final output 的值和对应 Runtime identity。普通 locator 只说明去哪里核查，
不证明所指内容保持不变；若结论依赖 locator 指向的内容，main Agent 必须在使用前核对实际内容与 result
声明的版本、摘要或不可变对象 identity。无法核对时保留为 uncertainty，不得把可变 locator 当作已冻结事实。
本蓝图到此为止；任何下游对象的构造、接纳与持久化由下游系统独立定义。

## 6. 角色输入输出模板

以下模板定义 main Agent 应提供和消费的语义数据。实际载体仍是当前 Runtime 支持的任务输入和最终输出。
各角色说明会显式指出必填集合；其他字段按任务相关性提供，可以为空或省略，不能为了满足模板制造无意义
内容。

### 6.1 `scout`

输入：

```json
{
  "roots": [
    {"id": "SR1", "locator": "本地路径、日志或数据入口", "purpose": "为什么检查这里"}
  ],
  "inspection_requests": [
    {
      "id": "SI1",
      "request": "需要定位、盘点、追踪或比较的事项",
      "hints": ["符号、文件或搜索入口"],
      "evidence_focus": ["实现、测试、配置或日志"]
    }
  ],
  "baselines": [
    {"id": "SB1", "locator": "可选的比较基线", "purpose": "比较目的"}
  ]
}
```

输出：

```json
{
  "root_results": [
    {
      "root_id": "SR1",
      "status": "inspected | no_finding | inaccessible | partial",
      "coverage": "实际检查范围",
      "evidence": []
    }
  ],
  "inspection_results": [
    {
      "request_id": "SI1",
      "status": "answered | no_finding | inaccessible | partial",
      "answer": "调查结果",
      "evidence": []
    }
  ],
  "facts": [
    {"id": "SF1", "claim": "直接观察", "root_ids": ["SR1"], "evidence": []}
  ],
  "inferences": [
    {"claim": "推断", "basis_fact_ids": ["SF1"], "limits": []}
  ],
  "conflicts": [
    {"subject": "互相冲突的本地事实", "evidence": []}
  ]
}
```

`roots` 和 `inspection_requests` 必填且非空。Scout 只读调查本地可见事实；每个 root 和 request 都返回
coverage status，包括无发现、不可访问或未完成。无法直接观察的内容不能伪装成 fact，推断必须引用其
本地事实依据。`roots` 是调查入口而不是路径 allowlist；Scout 可以沿与 objectives 相关的引用、依赖、测试、
配置和日志继续探索，并在 coverage 中说明实际检查范围。

### 6.2 `researcher`

输入：

```json
{
  "questions": [{"id": "RQ1", "question": "开放世界问题"}],
  "source_requirements": {
    "preferred_types": ["官方文档、标准、原始论文"],
    "freshness": "目标日期、版本或时效窗口",
    "authority_requirements": ["需要满足的来源权威性要求"]
  },
  "starting_points": [
    {"id": "RS1", "locator": "已知 URL、标准编号或检索入口", "purpose": "用途"}
  ],
  "comparison_dimensions": ["版本差异、来源冲突或方案比较维度"]
}
```

输出：

```json
{
  "sources": [
    {
      "id": "SRC1",
      "title": "来源标题",
      "locator": "URL、DOI、标准编号或文档位置",
      "publisher": "发布者",
      "published_or_version": "发布日期或版本",
      "accessed_at": "访问日期",
      "provenance": "primary | secondary | unknown",
      "authority_signals": [
        "official | standards_body | peer_reviewed | vendor | community | unknown"
      ]
    }
  ],
  "answers": [
    {
      "question_id": "RQ1",
      "answer": "回答或无法判断",
      "assessment": "direct | indirect | disputed | unknown",
      "source_ids": ["SRC1"],
      "evidence": ["具体章节、页码或段落位置"],
      "limits": []
    }
  ],
  "inferences": [
    {"claim": "跨来源推断", "source_ids": ["SRC1"], "limits": []}
  ],
  "source_conflicts": [
    {"subject": "冲突点", "source_ids": ["SRC1"], "assessment": "冲突如何影响结论"}
  ]
}
```

`questions` 必填且非空。Researcher 优先使用满足任务要求的原始和权威来源，核查时效与版本，并把无来源
的模型记忆视为待验证信息。每个问题都明确回答或标记无法判断；每个来源保留可核查 locator，来源数量
本身不替代来源质量。

### 6.3 `driver`

输入：

```json
{
  "change_targets": [
    {
      "id": "DT1",
      "target": "允许改变的文件、组件、数据或外部对象",
      "requested_change": "预期改变",
      "expected_outcome": "可观察结果"
    }
  ],
  "implementation_constraints": [
    {"id": "DC1", "constraint": "现有约定、可复用入口或必须保持的关系"}
  ],
  "validations": [
    {
      "id": "DV1",
      "check": "应执行的检查、观察或命令",
      "expected": "通过条件",
      "target_ids": ["DT1"]
    }
  ]
}
```

输出：

```json
{
  "target_results": [
    {
      "target_id": "DT1",
      "status": "changed | unchanged | partial | failed",
      "result": "实际结果",
      "effect_ids": ["E1"],
      "artifact_ids": ["A1"]
    }
  ],
  "commands": [
    {
      "id": "CMD1",
      "purpose": "为什么执行",
      "command": "已脱敏的命令",
      "exit_code": 0,
      "result": "关键输出摘要",
      "effect_ids": ["E1"],
      "validation_ids": ["DV1"]
    }
  ],
  "validation_results": [
    {
      "validation_id": "DV1",
      "status": "passed | failed | not_run | inconclusive",
      "actual": "实际观察",
      "evidence": []
    }
  ],
  "deviations": [
    {"subject": "相对请求或实现约束的偏离", "reason": "原因", "impact": "影响"}
  ]
}
```

`change_targets` 和 `validations` 必填且非空。每个 validation 都返回结果或未执行原因。普通修改由
objectives、change target、forbidden 和 Runtime 权限共同约束；若重大风险场景提供了 `allowed`，对应
动作还必须匹配其 allowlist。Driver 不返回第二份 `changes`：所有已尝试或实际发生的副作用只进入公共
`effects`。`commands` 是 provenance；有副作用的命令引用 effect ID，只读验证命令引用 validation ID，
任何 secret 或 credential 都必须脱敏。

### 6.4 `verifier`

输入：

```json
{
  "subjects": [{"id": "VS1", "subject": "待验证的文件、产物、接口或行为"}],
  "claims": [{"id": "VC1", "claim": "需要支持、反驳或标记为无法判断的主张"}],
  "checks": [
    {
      "id": "VK1",
      "check": "验证方法、观察或具体命令",
      "subject_ids": ["VS1"],
      "claim_ids": ["VC1"],
      "expected": "预期行为或比较基线",
      "counterexample": "能反驳预期的信号"
    }
  ],
  "environment": [
    {"id": "VE1", "condition": "版本、平台、fixture 或前置条件", "required": true}
  ]
}
```

输出：

```json
{
  "subject_results": [
    {
      "subject_id": "VS1",
      "status": "verified | contradicted | inconclusive | inaccessible | not_run",
      "evidence": []
    }
  ],
  "claim_results": [
    {
      "claim_id": "VC1",
      "assessment": "supports | contradicts | inconclusive | unknown | not_run",
      "evidence": []
    }
  ],
  "check_results": [
    {
      "check_id": "VK1",
      "status": "passed | failed | not_run | inconclusive",
      "actual": "实际观察",
      "environment_ids": ["VE1"],
      "evidence": []
    }
  ],
  "discrepancies": [
    {"subject_id": "VS1", "expected": "预期", "actual": "实际", "impact": "影响", "evidence": []}
  ],
  "gaps": [
    {"subject": "验证缺口", "reason": "原因", "needed": "补足条件"}
  ]
}
```

`subjects` 必填且非空；`claims` 和 `checks` 至少一项非空。Verifier 不修复被验证对象；临时测试副作用
必须在 envelope 中获得授权并进入公共 `effects`。每个 subject、claim 和 check 都逐项响应；`check` 可以
是观察、复现、比较或命令，不应为了满足模板而制造没有验证价值的命令。

### 6.5 `judge`

输入：

```json
{
  "materials": [
    {
      "id": "JM1",
      "locator": "可访问材料或内联内容",
      "purpose": "为什么纳入审查",
      "freeze": {
        "method": "inline | content_digest | immutable_version | immutable_object_id",
        "value": "固定内容、摘要、版本或不可变对象 identity"
      }
    }
  ],
  "questions": [
    {
      "id": "JQ1",
      "question": "需要独立审查的问题",
      "material_ids": ["JM1"],
      "required_coverage": "回答问题所需的完整范围"
    }
  ],
  "criteria": [
    {
      "id": "JC1",
      "criterion": "Judge 不得自行改写的评价标准",
      "material_ids": ["JM1"],
      "required_coverage": "支持判断所需的完整范围"
    }
  ],
  "known_risks": [
    {
      "id": "JR1",
      "risk": "必须主动挑战的疑点",
      "material_ids": ["JM1"],
      "required_coverage": "评估风险所需的完整范围"
    }
  ],
  "disposition_options": ["允许推荐的处置选项"]
}
```

输出：

```json
{
  "material_results": [
    {
      "material_id": "JM1",
      "status": "reviewed | stale | unverifiable | inaccessible | partial",
      "freeze_check": {
        "status": "matched | mismatched | unverifiable",
        "observed": "实际观察到的摘要、版本、不可变对象 identity 或无法核查原因"
      },
      "coverage": "实际审查范围",
      "evidence": []
    }
  ],
  "answers": [
    {
      "question_id": "JQ1",
      "assessment": "answered | inconclusive",
      "answer": "回答或无法判断",
      "material_ids": ["JM1"],
      "coverage_status": "complete | partial | unverifiable",
      "evidence": []
    }
  ],
  "criterion_assessments": [
    {
      "criterion_id": "JC1",
      "assessment": "satisfied | unsatisfied | inconclusive",
      "material_ids": ["JM1"],
      "coverage_status": "complete | partial | unverifiable",
      "evidence": [],
      "reason": "判断依据"
    }
  ],
  "risk_assessments": [
    {
      "risk_id": "JR1",
      "assessment": "observed | mitigated | unsupported | inconclusive",
      "material_ids": ["JM1"],
      "coverage_status": "complete | partial | unverifiable",
      "evidence": []
    }
  ],
  "findings": [
    {
      "severity": "minor | major | blocking",
      "finding": "问题或反证",
      "criterion_ids": ["JC1"],
      "material_ids": ["JM1"],
      "evidence": []
    }
  ],
  "recommended_disposition": "disposition_options 中的一个选项，或 inconclusive",
  "recommendations": [
    {"recommendation": "建议", "reason": "原因", "evidence": []}
  ]
}
```

`materials`、`questions` 和 `criteria` 必填且各自非空。每个 question、criterion 和 known risk 都必须列出
完成判断所需的 `material_ids` 与 `required_coverage`。每个 material 都必须携带 freeze 声明；locator
本身不是冻结机制。`inline` 固定任务内联值，其他方法分别固定内容摘要、不可变版本或稳定对象 identity。
具体算法和格式由 main Agent 与当前工具能力决定，本蓝图不创建统一摘要协议。

Judge 在进行实质审查或把材料引用为依据前逐项核对 freeze。只有 `matched` 的材料可以支持 criterion
assessment；`mismatched` 标记
为 `stale`，无法核对则标记为 `unverifiable`。两者都不能被当作已审查的冻结内容，也不能通过重新读取
当前 live 内容悄然替换任务指定版本。辅助背景不能扩展证据集。每个 material、question、criterion 和
known risk 都逐项响应。任何确定性的 answer、criterion assessment 或 risk assessment 只能引用 freeze
`matched` 且达到 `required_coverage` 的全部必要材料。必要材料为 `partial` 时，对应
`coverage_status=partial`；必要材料为 `stale`、`unverifiable` 或 `inaccessible` 时，对应
`coverage_status=unverifiable`；两类情况的 assessment 都必须为 `inconclusive`。
存在 `disposition_options` 时，推荐必须来自该集合；任一 question、criterion 或 known risk 因必要材料
不完整而为 `inconclusive` 时，整体 recommended disposition 也必须为 `inconclusive`。所有输出保持 advisory。
`answers`、`risk_assessments`、`findings` 和 `recommendations` 不能替代或绕过上述 criterion assessment 与
disposition gate；它们引用非完整材料时只能描述缺口与无法判断，不能表达确定结论。

## 7. Effect 声明

任何获准产生副作用的角色都使用同一格式；Driver 对每个已尝试或实际发生的副作用至少声明：

```json
{
  "id": "E1",
  "target_ref": "DT1、VS1 或其他角色内目标 ID",
  "target": "受影响对象",
  "operation": "created | modified | deleted | executed | external_action",
  "authorization": {
    "status": "authorized | unauthorized | ambiguous",
    "refs": ["O1", "DT1", "高风险场景下适用的 AL1"]
  },
  "status": "completed | partial | failed | rolled_back",
  "before": "可得时记录变化前状态或基线",
  "after": "实际观察到的结果",
  "provenance": ["command ID、工具调用或官方外部对象 ID"],
  "verification": ["检查、命令结果或 artifact locator"],
  "unexpected": [],
  "residual_risks": [],
  "cleanup": {
    "status": "not_needed | completed | pending",
    "reason": "状态原因",
    "responsible": "main Agent 或稳定外部责任方",
    "evidence": []
  }
}
```

产生 effect 的 subagent 必须：

1. 只主动选择为完成 objectives 和 role input 所合理需要、未被 forbidden 禁止且 Runtime 权限允许的动作；
2. 若任务因重大风险提供了 `allowed`，相应动作还必须匹配其 positive allowlist；
3. 报告成功、失败、部分完成、回滚和已经发生的未授权或授权不明确影响；
4. 对文件给出路径，对命令关联 `command ID` 和退出状态，对外部动作给出官方对象标识或可核查 locator；
5. 验证每项 effect 的实际结果；无法验证时明确形成 gap；
6. 说明意外影响、共享工作区冲突、残余风险和待清理对象；
7. 将 `cleanup=pending` 的责任移交给 main Agent 或可跨任务持续存在的外部责任方；one-shot subagent
   不能成为后续责任人；
8. 排除 secret、credential 和无关敏感内容；
9. 不把“操作完成”表达为任何下游业务结果已被接受。

未授权但已经发生的 effect 仍必须声明为 `unauthorized`；无法从 objectives、role input 和适用的高风险
allowlist 确认时声明为 `ambiguous`。声明义务不构成事后授权。main Agent 收到结果后检查实际 diff、命令
结果、官方外部对象和未预期影响，再决定接纳、验证、修复、回滚、重试、换路或停止。

## 8. Judge advice 与 main Agent 判断

Judge 接收冻结的审查材料、问题和评价标准，主动寻找反例、来源冲突、缺失证据和替代解释。它返回逐项
assessment、证据位置、findings、严重性、推荐 disposition 和未解决问题。

Judge 不返回下游系统的正式 Decision，也不调用其记录或状态转移接口。main Agent 检查 freeze verification、
实际 coverage、Judge advice 和其他反证后，独立决定是否以及如何把这些意见转义给下游系统。

需要多个模型视角时，main Agent 先定义一个总 result budget，并让每个新增 Judge 覆盖尚未覆盖的具体问题、
失效模型或反证面。各 Judge 独立返回，main Agent 只生成一份去重综合；重复的全文、材料清单与发现通过
locator 关联，不在每份输出或综合中再次复制。票数、模型数量、Runtime success 或 Judge 推荐都不能替代
main Agent 的语义判断；没有新增信息价值的 fanout 不应启动。

## 9. 调用与生命周期

1. main Agent 识别当前缺少的调查、执行、验证或审查结果；
2. 选择角色、模型 policy 和副作用边界，构造完整 basic envelope；
3. 内联公共输出、所选角色输出以及适用的 effect 格式，通过当前 host 的原生 Subagent workflow 创建
   agent thread 并发送任务；Driver 不引入另一套执行 Runtime；
4. main Agent 可以继续处理不依赖该结果的工作，需要结果时等待；
5. 直接消费 Runtime 返回的最终输出、items、状态和 usage；
6. 按 Runtime 正常生命周期关闭已完成、失败或不再需要的执行；
7. 检查 delegation baseline、材料版本、effect scope、provenance 和未知项，再决定如何消费结果；
8. 需要补充、重试或改换角色时默认创建新任务，不用旧 subagent thread 承担业务连续性。

Codex 可以在当前任务内路由 follow-up 或停止失控线程。这些线程控制不转移结果解释权，也不把 Runtime
线程变成持久业务 actor。follow-up 仅用于同一 envelope、baseline 与授权边界内的澄清或补齐；目标、角色、
授权、冻结材料或 baseline 改变时创建新任务。

## 10. 并发与串行接纳

- 只有输入、工作范围和副作用互相独立时才并发。
- 多个只读调查可以并发；多个 Judge 可以独立审查同一冻结材料。
- 修改范围重叠或可能竞争同一外部对象的 Driver 串行执行。
- Verifier 在所验证的 Driver effects 已发生并稳定后启动。
- subagent 可以并发工作，main Agent 可以并行检查彼此独立的结果，但按各下游系统自己的规则串行接纳和提交。
- fanout 共享一个有限总 result budget；每个 child 使用独立子预算，main Agent 只保留一次综合和去重 locator。
- 返回时 delegation baseline 或冻结材料版本已改变的结果不能按原任务前提消费；main Agent 可以把它保留
  为线索，或基于新 baseline 创建新任务。已经发生的 effect 不会因结果过期而自动撤销，仍需核查和清理。

## 11. Runtime、模型与 provider 边界

- Runtime 对 agent/thread、turn、item、工具调用、权限、状态、模型和 usage 的公开对象是唯一运行时事实。
- 本 skill 不复制官方类型，不推测未公开字段，也不把某个 CLI 版本的内部实现提升为 skill 契约。
- Driver 只使用当前 host 正式支持的原生 Subagent workflow；线程创建、上下文装配和生命周期由 Runtime 负责。
- Driver 是语义角色，不要求固定或证明 Runtime agent identity；实际 spawn、配置、Runtime 或权限错误如实失败。
- Driver 不固定专属 model 或 reasoning；Runtime 负责解析，并应用官方定义的 sandbox 与 permission 继承。
- Provider、model catalog、custom-agent 安装、用户配置修改和重启验证属于独立、显式授权的 setup 工作。
- skill 不能扩大 Runtime 权限。高风险、不可逆或影响外部人员和系统的动作仍遵守 host approval、sandbox
  和用户授权边界。
- Runtime 即使向 subagent 暴露 downstream Composition 明确禁止的工具，也不改变 main Agent 的语义
  ownership；subagent 依照 Skill 和任务 envelope 不使用这些工具。本 skill 不建立 main/subagent caller
  认证或 per-thread tool 隔离。

## 12. Skill packaging 与 Pruning 原则

`mobius-subagent` 应保持为可按需发现、instruction-first 的薄 skill：

- `SKILL.md` 只保留不变量、角色/模型选择、Driver 原生调用、任务构造和消费检查；
- 第 6 节的完整角色模板放入一层 `references/role-profiles.md`，按所选角色读取；
- 只有确定性校验确有必要时才增加 script；
- provider/setup 若需要自动化，使用独立 skill 或安装组件；
- 不创建独立 subagent ledger、任务队列、scheduler、registry、heartbeat、memory 或 Runtime schema
  mirror；
- 不把模板演化成与 Codex 官方对象竞争的 transport protocol；
- 用户不需要单独显式调用本 skill；main Agent 在当前工作确实需要有界调查、执行、验证或独立审查时可以
  按需选择它。Skill 的可发现性本身不授权副作用，也不改变用户授权、Runtime permission 或 task envelope
  的边界。

新增概念必须回答：它是否表达 Runtime 没有拥有、而委托工作流确实需要的语义？如果只是重复官方状态
或证明某次官方调用发生过，就直接消费官方对象，不进入 skill。

## 13. 验收条件

1. main Agent 是委托编排、结果解释和任何下游提交的唯一 owner。
2. Runtime 官方对象是运行时事实的唯一来源，skill 不建立平行控制面或下游状态模型。
3. 通用 Skill package 不包含任何 downstream-specific API、path 或 schema knowledge；更窄的 ownership 边界
   只由 Composition 写入每次任务的 `forbidden`。
4. Driver 只有当前 host 的原生 Subagent workflow 一条执行路径；不要求 Runtime agent identity attestation，
   实际 spawn、配置、Runtime 或权限错误如实失败。
5. 每次启动都包含 background、一个或多个 objectives、forbidden-first boundaries、带有限正整数
   `result_budget.max_public_result_bytes` 的完整 output format 和 DONE conditions，并与所选角色的
   input/output format 组合。
6. 默认不提供 `allowed`；focus、roots 和已列路径不限制相关自主探索。只有重大风险场景才增加穷举式
   positive allowlist。
7. 模型选择矩阵与五类角色的输入输出模板完整保留，并作为语义指导而非自定义传输协议。
8. Driver 可以执行 objectives 和 change targets 所需、用户已授权、未被 forbidden 禁止且 Runtime 权限
   允许的副作用；高风险 `allowed` 存在时还必须匹配它，并完整声明实际、失败、意外、越界和待清理影响。
9. subagent 结果只提供候选 observation、effect、artifact、advice 和 provenance，不直接取得下游事实地位。
10. Judge 只使用 freeze 已匹配且 coverage 完整的材料作任何确定性判断，并只产生 advice；必要材料 partial、
   stale、unverifiable 或 inaccessible 时，对应 question、criterion、risk 与整体 disposition 都返回
   `inconclusive`，其他字段不得成为旁路。
11. main Agent 在消费结果前验证 delegation baseline、材料版本、effect scope、provenance 和未知项。
12. subagent 工作可以并发，main Agent 可以并行检查独立结果，但串行接纳和提交下游状态。
13. result envelope 整体遵守序列化 byte budget、去重并显式报告 overflow；单个 item 不能绕过上限，
    大型或原始内容只以可核查 artifact locator 返回。Judge fanout 覆盖不同问题或失效模型，并共享有限
    总预算，main Agent 只生成一份综合。
14. 模型 policy 随官方能力演进；skill 不固定 Runtime 内部实现，也不以 per-thread tool hiding 或 caller
    attestation 作为可用前提。main/subagent 采用协作式 Agent 信任边界，恶意或被攻陷 Agent 不在威胁模型内。
