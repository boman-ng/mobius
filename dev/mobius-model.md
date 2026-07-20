# Mobius 数学模型

## 0. 模型目的

Mobius 是一个由 main Agent 驱动的、严格串行、可审计、可恢复的动态寻路系统。

它用持续修正认知的 Map 取代机械执行的 Plan：人类给出 Objective，Map 标记 Stage 与已知 Route，main Agent 通过 Attempt 获取 Evidence，并以正式 Reviewer 身份对冻结的证据作出 Judgment；系统据此继续尝试、修复、换路、等待或重绘 Map，直到 Objective 被证明达成或由人类放弃。开放世界中的调查、执行与审查如何产生候选输入不属于本模型；这些输入只有经 main Agent 转义为合法的模型对象并通过状态转移前提后，才拥有 Mobius 事实地位。

唯一主循环是：

\[
Objective
\to Stage
\to Route
\to Attempt
\to Evidence
\to Review
\to
\begin{cases}
Stage\ Achieved,\\
Retry,\\
Replace\ Route,\\
Wait,\\
Remap.
\end{cases}
\]

重复该循环，直到所有 Stage 都有当前有效的验收判断，才能宣称 Objective 完成。

本文件覆盖 Mobius 的理论对象、状态、转移关系、不变量与活性边界。持久化方式、数据库、序列化、命令接口、外部工作 transport、进程身份、部署和安全加固留给实现规格。

---

## 1. 设计公理

### 1.1 Objective 是稳定终点

Objective 的身份在整个生命周期中保持不变。人类可以修订它的可操作化描述。修订保留原 Objective 的完成责任。

### 1.2 Map 表达动态认知

Map 表达当前已知的 Stage 拓扑、验收分工和 Route 知识。新事实可以扩充 Route 知识；当新事实改变 Stage、验收条件或依赖关系时，系统产生新的 MapRevision。

### 1.3 Stage 严格串行

Stage 之间可以形成 DAG。任一时刻至多存在一个 current Stage。DAG 决定合法先后关系，串行 cursor 决定实际执行顺序。

### 1.4 Route 是可证伪假设

Route 表达关于“如何达成当前 Stage”的实现假设，与任务清单保持分离。每次 Attempt 检验一条 Route；main Agent 可以要求在同一 Route 上修复重试，也可以判定应当换路。

### 1.5 Evidence 是观察，Decision 是 main Agent 的正式判断

Evidence 可以支持、反驳或无法判定某个 Criterion。程序保证 main Agent 的 Reviewer 角色收到完整且冻结的证据集合；main Agent 可以消费模型外的 advisory input，但正式 ReviewDecision 只能由 main Agent 给出。判断的语义真值留在开放世界中。

每个 Stage Review 都包含固定 Judge 仪式：main Agent 完成 Packet 的递归 identity closure、artifact integrity、
Evidence applicability 与材料冻结后，必须通过通用 Subagent Judge 合同取得一次针对该冻结 Review 的新鲜独立
质疑，再形成正式 ReviewDecision。Judge 始终只提供 advisory input；该仪式既不把 Judge 变成 Mobius actor，
也不转移 main Agent 的裁决责任。

### 1.6 Agentic 输出经过 guard 进入事实

main Agent 可以提出 ObjectiveSpec、Map、Route、Evidence 解释和下一步建议，也可以消费来自任意模型外来源的观察、外部影响声明、产物或 advisory input。模型外输出始终只是候选输入；main Agent 必须把其中可接纳的内容转义为本模型定义的完整对象，且只有满足状态转移前提的对象才能进入新的系统状态。语义判断归属 main Agent 承担的 Agentic 职责，状态守恒归属 Programmatic。

### 1.7 Trail 承载唯一历史

系统状态由初始状态和不可变 Trail 唯一派生。当前状态充当历史投影，Trail 独占事实所有权。

---

## 2. 核心类型

模型保留以下十一类一等对象：

\[
\begin{aligned}
\mathcal J &:= \text{Objectives}, &
\mathcal O &:= \text{ObjectiveSpec revisions}, &
\mathcal M &:= \text{Map revisions},\\
\mathcal S &:= \text{Stages}, &
\mathcal C &:= \text{Criteria}, &
\mathcal R &:= \text{Routes},\\
\mathcal A &:= \text{Attempts}, &
\mathcal E &:= \text{Evidence}, &
\mathcal P &:= \text{ReviewPackets},\\
\mathcal D &:= \text{ReviewDecisions}, &
\mathcal B &:= \text{WaitConditions}. &&
\end{aligned}
\]

Trail 由有限状态转移事实组成。派生 proof 与证据视图无需独立实体。

所有一等对象一旦进入 Trail 就不可原地修改。变化通过新 revision、新 Attempt、新 Decision 或新状态转移表达。

令 \(\Omega_j\) 表示 Objective \(j\) 截至某个 Trail 位置已经接纳的一等对象有限集。每个配置读取其 Objective-scoped \(\Omega_j\)，后文省略无歧义的下标并简写为 \(\Omega\)。Route、Evidence 和 proof 查询都从该集合派生。

令 \(\mathcal U\) 为上述一等对象类型的并集，\(\mathcal I\) 为理论 identity 集，并定义：

\[
id:\mathcal U\to\mathcal I,
\qquad
\forall x,y\in\mathcal U:\ id(x)=id(y)\iff x=y.
\]

模型将 identity 唯一性作为公理。Hash、碰撞分支与防御机制位于模型边界之外。新对象进入知识集的统一前提为：

\[
Fresh_\Omega(x)\iff x\notin\Omega.
\]

原子转移一次接纳有限批次 \(X\) 时：

\[
FreshBatch_\Omega(X)
\iff
X\text{ 有限}\land X\cap\Omega=\varnothing.
\]

已有对象可以被后续 revision 精确引用；集合并集不会再次接纳它们。

---

## 3. Objective、Criterion 与 Map

### 3.1 Objective 与 ObjectiveSpec

Objective 是稳定身份：

\[
j\in\mathcal J.
\]

ObjectiveSpec revision 将人类意图操作化：

\[
O_j^\nu=(j,\nu,I,C^\nu,K,N)\in\mathcal O,
\]

其 identity 由类型、Objective identity 与 revision 共同确定：

\[
id(O_j^\nu)=(ObjectiveSpec,id(j),\nu).
\]

其中：

- \(I\) 是期望结果；
- \(C^\nu\subset\mathcal C\) 是有限、非空的 Objective 验收条件集合；
- \(K\) 是必须遵守的边界；
- \(N\) 列出明确排除的声称；
- \(\nu\) 是 revision。

改变 \(I\)、\(C^\nu\)、\(K\) 或 \(N\) 都产生新的 ObjectiveSpec revision，并需要人类授权。Agent 不能通过自行缩减 Criterion 来制造完成。

记 \(H^{confirm}\) 为人类对某个 ObjectiveSpec revision 的确认判断。它随 Activate 或 Revise 输入进入 Trail。业务实体集合保持不变。该判断是 main Agent 在人类明确确认完整 revision 后构造的语义 transition input；程序检查它与当前 ObjectiveSpec 的精确绑定，不要求 Runtime principal attestation、签名或调用线程认证。

### 3.2 Criterion

Criterion 是一个由 main Agent 正式判断的必要条件：

\[
c=(statement,verificationRule,scope)\in\mathcal C,
\]

其中：

\[
scope(c)\in\{local,cross\_stage\}.
\]

模型采用统一 Criterion 类型。ObjectiveSpec 定义 Objective 必须满足的 Criterion；Map 可以加入实现当前 Stage 所需的局部 Criterion。两者共享同一类型。

### 3.3 Stage

Stage 是一个具有可观察结果和输出契约的里程碑：

\[
s=(identity,name,outcome,output,kind)\in\mathcal S,
\]

其中 \(identity(s)=id(s)\)。

其中：

\[
kind(s)\in\{ordinary,final\_integration\}.
\]

`outcome` 描述本 Stage 完成后应当成立的事实；`output` 描述下游 Stage 可以依赖的内容。两者均属于理论契约，产物存储形式留给实现规格。

### 3.4 MapRevision

MapRevision 定义当前的结构认知：

\[
M^\mu=(O_j^\nu,S_\mu,\widehat C_\mu,E_\mu,\pi_\mu,owner_\mu,Contract_\mu)
\in\mathcal M.
\]

其 identity 由类型、Objective identity 与 revision 共同确定：

\[
id(M^\mu)=(MapRevision,id(j),\mu).
\]

其中：

- \(S_\mu\subset\mathcal S\) 是有限、非空 Stage 集；
- \(\widehat C_\mu\subset\mathcal C\) 是该 Map 使用的有限 Criterion 集，且 \(C^\nu\subseteq\widehat C_\mu\)；
- \(G_\mu=(S_\mu,E_\mu)\) 是 Stage 依赖 DAG；
- \(\pi_\mu:S_\mu\to\mathbb N\) 是稳定优先级；
- \(owner_\mu:\widehat C_\mu\to S_\mu\) 为每个 Criterion 指定唯一验收 Stage；
- \(Contract_\mu(s)\) 是 Stage \(s\) 在该 Map 下的局部验收契约。

边 \((d,s)\in E_\mu\) 表示 Stage \(s\) 依赖 Stage \(d\)。直接依赖与 transitive dependencies 分别定义为：

\[
Dep_\mu(s):=\{d\in S_\mu\mid(d,s)\in E_\mu\},
\qquad
Dep_\mu^+(s):=\{d\in S_\mu\mid d\leadsto s\text{ in }G_\mu\}.
\]

局部 Criterion 集为：

\[
C_\mu(s):=\{c\in\widehat C_\mu\mid owner_\mu(c)=s\}.
\]

其中 \(Contract_\mu(s)\) 至少包含 \(s\) 的 outcome、\(C_\mu(s)\)、所有被 Map 判定会影响 \(s\) 的 Objective 边界，以及 \(s\) 的 output。是否相关由制图过程判断；一旦写入 Map，它就是后续 carry 的显式语义边界。

有效 Map 必须满足：

\[
\operatorname{Acyclic}(G_\mu),
\qquad
\forall s\in S_\mu:\ C_\mu(s)\ne\varnothing.
\]

并且必须存在显式的制图判断：

\[
Cover(M^\mu,O_j^\nu),
\]

表示这些 Stage contract、Criterion ownership 和依赖关系共同覆盖了当前 ObjectiveSpec 的结果与边界。`Cover` 属于 Agentic judgment；程序检查它的存在性与 revision 绑定，意图理解质量留给制图者和 main Agent 的 Reviewer 角色。

该判断作为 Map 安装输入 \(J^{cover}\) 进入 Trail，避免隐藏在 Map 字段或程序推断中：

\[
J^{cover}=(M^\mu,O_j^\nu,covered,rationale).
\]

\(J^{cover}\) 同样由安装转移记录，无需纳入一等对象。

Criterion 可以在同一个 Stage 中共同验收。Objective Criterion 与 Stage-local Criterion 使用同一种判断和证据语义，不需要在两套类型之间建立绑定。

### 3.5 Final Integration Stage

Map 至多存在一个 final integration Stage：

\[
|\{s\in S_\mu\mid kind(s)=final\_integration\}|\le1.
\]

若存在 cross-stage Criterion，则该 Stage 必须存在，记为 \(s_\star\)，并满足：

\[
\forall c\in\widehat C_\mu:
scope(c)=cross\_stage
\Rightarrow owner_\mu(c)=s_\star,
\]

任何存在的 final integration Stage 都必须覆盖所有 ordinary Stage：

\[
Dep_\mu^+(s_\star)=S_\mu\setminus\{s_\star\}.
\]

因此全局集成验收仍然走普通的 Route—Attempt—Evidence—Review 循环，不存在隐藏的 Objective Exit Review。

### 3.6 动态 Route 知识

当前 Mobius Map 同时包含结构 revision 与动态 Route 知识：

\[
MapState=(M^\mu,KnownRoutes),
\]

其中：

\[
KnownRoutes_\Omega(s):=
\{r\in\Omega\cap\mathcal R\mid stage(r)=s\}.
\]

它保留成功、失败和历史 Context 下的 Route 知识。可选 Route 需要同时满足 `available` status 与当前 Structural Context。

新增、证伪或替换 Route 更新 Route 知识，Stage 拓扑保持原值。Stage、Criterion ownership、局部契约或依赖关系的变化触发新 MapRevision。

---

## 4. Acceptance Context 与证明继承

### 4.1 Structural Context 与 Acceptance Context

先定义描述契约结构的 Structural Context：

\[
\kappa_\mu(s)=\Big(
Contract_\mu(s),
\{(d,output(d),\kappa_\mu(d))\mid d\in Dep_\mu(s)\}
\Big).
\]

由于 \(G_\mu\) 无环，\(\kappa_\mu\) 可以按拓扑序唯一计算。它不会因为与 \(s\) 无关的 ObjectiveSpec 文案或其他 Stage 改动而失效。若某项全局约束确实影响 \(s\)，它必须进入 \(Contract_\mu(s)\)。

当且仅当 \(s\) 的直接依赖都有当前 proof 时，定义运行时 Acceptance Context：

\[
\chi_{\mu,q}(s)=\Big(
\kappa_\mu(s),
\{(d,Proof_{\mu,q}(d))\mid d\in Dep_\mu(s)\}
\Big).
\]

因此 Route 可以在依赖完成前基于 Structural Context 预先设计；真正开始 Attempt 时，Acceptance Context 还会冻结本次实验实际依赖的验收结果。

### 4.2 当前 Stage proof

一个被合法接受、且仍与当前 Acceptance Context 相符的 ReviewDecision 直接充当 Stage proof。

定义：

\[
Proof_{\mu,q}:S_\mu\rightharpoonup\mathcal D,
\qquad
Proof_{\mu,q}(s)=D
\]

当且仅当：

1. \(D\in\Omega\cap\mathcal D\)；
2. \(action(D)=accept\)；
3. \(D\) 复核的 Stage 是 \(s\)；
4. \(D\) 的 Packet context 等于 \(\chi_{\mu,q}(s)\)；
5. \(D\) 对 \(C_\mu(s)\) 中每个 Criterion 的判断都是 `satisfied`；
6. \(D\notin InvalidatedProofs(q)\)。

每个当前 \((M^\mu,s)\) 至多有一个 \(Proof_{\mu,q}(s)\)。

### 4.3 Remap carry

Context 相同提供 proof 的结构继承资格。开放世界中的事实有效性仍需显式语义判断。先定义结构资格：

\[
StructuralEligible_q(s,\mu,\mu')
\iff
s\in S_\mu\cap S_{\mu'}
\land \kappa_\mu(s)=\kappa_{\mu'}(s)
\land Proof_{\mu,q}(s)\text{ 有定义}.
\]

结构 eligible Stage 集合为：

\[
EligibleStages_q(\mu,\mu')
:=
\{s\in S_{\mu'}\mid StructuralEligible_q(s,\mu,\mu')\}.
\]

Map 安装输入提供一个全函数：

\[
J^{carry}_{q,\mu\to\mu'}:
EligibleStages_q(\mu,\mu')
\to\{valid,invalid\}.
\]

`J^{carry}` 的 `valid` 是 main Agent 对旧 proof 当前语义有效性的显式判断，不是结构相等的别名。若模型外
工作成果、工具链、共享配置、migration、并发/lease、filesystem/security boundary 或外部对象版本已经变化，
main Agent 必须先核对旧 Evidence 固定的材料版本与变化影响；无法证明仍适用时给出 `invalid`。该判断留在
Agentic boundary，结构 eligibility 与依赖闭包仍由程序机械验证。

最终继承条件在 Stage identity 集 \(\mathcal S\) 上全域定义：

\[
Carry_q(s,\mu,\mu') :=
\begin{cases}
J^{carry}_{q,\mu\to\mu'}(s)=valid\\
\quad\land\ \forall d\in Dep_{\mu'}(s):Carry_q(d,\mu,\mu')\\
\quad\land\ TreeCompatible_q(s,\mu,\mu'),
&s\in EligibleStages_q(\mu,\mu'),\\[0.5em]
false,&s\notin EligibleStages_q(\mu,\mu').
\end{cases}
\]

其中：

\[
\begin{aligned}
TreeCompatible_q(s,\mu,\mu')\iff{}&kind(s)\ne final\_integration\\
&\lor\ DependencyView(packet(Proof_{\mu,q}(s)))\\
&\quad=\{Proof_{\mu,q}(d)\mid
d\in Dep_{\mu'}^+(s)\land Carry_q(d,\mu,\mu')\}.
\end{aligned}
\]

依赖按新 Map 的拓扑序先行判断，因而 \(Carry\) 与 \(TreeCompatible\) 构成良定递归。依赖 carry 失败会使 dependent 同步失去继承资格。final integration Stage 的集合等式要求旧 Packet proof tree 与新 Map 中已继承的 transitive dependency proofs 精确一致。

对于成立的 \(Carry_q(s,\mu,\mu')\)，新配置 \(q'\) 满足 \(Proof_{\mu',q'}(s)=Proof_{\mu,q}(s)\)。其余 Stage 在新 Map 中待重新验收。

Remap carry 复用已经成立的语义判断。旧状态标签留在历史 Trail 中。

---

## 5. Route、Attempt 与 Evidence

### 5.1 Route

Route 是对当前 Stage 实现机制的可证伪假设：

\[
r=(stage,structuralContext,hypothesis,assumptions,rationale)
\in\mathcal R.
\]

有效 Route 必须满足：

\[
stage(r)=s,
\qquad
structuralContext(r)=\kappa_\mu(s).
\]

queued Stage 的预设计 Route 可以随 \(R^0\) 在 Map 安装时进入知识集。运行中的 `AddRoute` 扩充范围限定为 current Stage，从而保持串行决策边界。

Route 保留“当前可选性”这一项 lifecycle 投影：

\[
routeStatus(r)\in\{available,rejected\}.
\]

`rejected` 表示 main Agent 已通过正式的 route-invalidating Judgment 判定该假设在其 Structural Context 下失效。合法来源仅有两种：

1. `ReviewDecision` 的 `action=replace`，拒绝该 Decision 所复核 Attempt 的 Route；
2. `CheckWait` 的 \(J_b.direction=new\_route\)，拒绝当前 `Waiting(s,r,b)` 中的 Route \(r\)。

两者都是由 main Agent 给出、经对应 transition guard 接纳并记录在 Trail 中的正式判断。`route-invalidating Judgment` 只是对这两类既有 transition input 的共同语义分类，不增加一等对象、Judgment schema 或状态转移。问题局限于当前 dependency proof、暂时外部条件或 Map 认知漂移时，main Agent 应选择 `retry`、`wait` 或 `remap`，这些方向都保持 Route status。当前选中关系由 NavState 表达，Route rejection 由上述两类 Trail 事实表达，Structural Context 适用性由 \(\kappa_\mu\) 比较表达。

### 5.2 Attempt

Attempt 是一条 Route 上的一次有界实验：

\[
a=(route,ordinal,bound,context)\in\mathcal A.
\]

其中 \(bound\) 表示本次实验预先声明的停止边界，可以采用资源预算、验证范围或明确终止条件。具体计时机制留给实现规格。开始 Attempt 时必须冻结：

\[
context(a)=\chi_{\mu,q}(stage(route(a))).
\]

Attempt 的理论状态集含三个成员：

\[
attemptState(a)\in\{running,sealed,closed\}.
\]

在同一 Map 与 Context 中结束实验时，所有成功、失败、达到边界或中断的结果都先 Seal，再进入 Review：

\[
sealReason(a)\in
\{submitted,bound\_reached,interrupted\}.
\]

失败、超时和中断都进入 Review。相关观察写入 Evidence，终止类别写入 Packet 的 \(termination\) 字段。Remap、ObjectiveSpec revision 或 Abandon 可以直接关闭尚未 Seal 的 Attempt，因为原实验的验收 Context 已失效或 Objective 已终止。

\[
closeReason(a)\in\{reviewed,remapped,abandoned\}.
\]

\(sealReason\) 同时冻结到 Packet；\(closeReason\) 从相应 Trail 事实派生。后续 guard 的状态依赖限定为 \(attemptState\)。

### 5.3 Evidence

Evidence 表达一次不可变观察：

\[
e=(subject,context,purpose,claims,observation,provenance)\in\mathcal E,
\]

其中：

\[
subject(e)\in\mathcal A\cup\mathcal B,
\qquad
purpose(e)\in\{stage\_review,wait\_resolution\}.
\]

若 \(purpose(e)=stage\_review\)，则 \(subject(e)\) 必须是当前 running Attempt，且：

\[
claims_e:C_\mu(stage(route(attempt(e))))
\rightharpoonup
\{supports,contradicts,unknown\}.
\]

这类 Evidence 必须绑定该 Attempt 的 Acceptance Context，可以反驳 Route 或 Criterion；反证与未知结果保留在 Evidence 中。`unknown` 表示已有观察且判断依据仍不充足；partial function 中缺省表示该 Evidence 没有评价该 Criterion。

若 \(purpose(e)=wait\_resolution\)，则 \(subject(e)\) 必须是当前 WaitCondition，Context 必须与该 WaitCondition 相同，且 \(claims(e)=\varnothing\)。这类 Evidence 的观察范围限定为外部等待条件的变化。

Stage acceptance 使用 `stage_review` Evidence。`wait_resolution` Evidence 服务于解除等待；同一事实与 Stage 验收有关时，它需在恢复后的 Attempt 中作为 `stage_review` Evidence 明确记录。

模型外来源不产生 Evidence。main Agent 必须先把候选观察转义为完整的 \(e\)，再请求接纳。该转义不保留模型外任务、角色、消息或线程语义；这些内容至多作为 provenance 的一部分出现。

定义 Evidence payload 的冻结条件：

\[
FrozenEvidence(e)
\]

当且仅当 `claims`、`observation` 与 `provenance` 都作为 \(e\) 的值被固定，且后续无需解引用一个可变位置才能确定这次观察声称了什么。Locator 可以作为 provenance，但不能单独充当 observation。若观察依赖外部 artifact，\(e\) 必须固定被观察内容，或固定足以区分被观察版本的稳定内容身份；具体快照、版本或摘要机制属于实现规格。

对于依赖可变工作成果、命令输出或外部对象的观察，稳定版本身份必须同时覆盖实际验收范围、验证前后材料、
验证方法/结果、已观察 effect、反证与覆盖限制。只有验证前后材料身份一致的观察，main Agent 才能把它转义为
当前 Attempt 的候选 Evidence；版本只存在于 prose、时间戳或可变 locator 时不满足 `FrozenEvidence`。

历史 Evidence 一旦进入 Trail 就保持原值。设开放世界当前观察为 \(w\)，main Agent 在正式 Review 时对每条
依赖可变材料的 Evidence 给出模型外适用性分类：

\[
MaterialApplicability_w(e)\in
\{current\_applicable,superseded,unverifiable\}.
\]

- `current_applicable`：冻结观察有效、验证前后同版，且同一范围的当前材料身份仍等于已验证后态；
- `superseded`：历史冻结观察有效，但当前材料已经是另一版本；
- `unverifiable`：schema、身份、完整性、范围或当前捕获不足以判定。

该分类依赖开放世界输入，不进入 \(q\)、Trail、projection、reducer 或 `EvidenceAdmission`。材料变化不修改、删除或
标记旧 Evidence；需要当前证明时，main Agent 在 running Attempt 中重新验证并追加新 Evidence，或在 Review/
后续 Stage 中走既有 retry/remap 生命周期。

给定接纳前配置 \(q\)，定义统一接纳谓词：

\[
EvidenceAdmission_q(e)
\iff
Fresh_{\Omega(q)}(e)
\land FrozenEvidence(e)
\land
\begin{cases}
ObjectiveState(q)=Navigating(j,M^\mu,Attempting(s,r,a))\\
\quad\land attemptState(a)=running\\
\quad\land purpose(e)=stage\_review\\
\quad\land subject(e)=a\\
\quad\land context(e)=context(a)=\chi_{\mu,q}(s)\\
\quad\land domain(claims_e)\subseteq C_\mu(s),
&subject(e)\in\mathcal A,\\[0.8em]
ObjectiveState(q)=Navigating(j,M^\mu,Waiting(s,r,b))\\
\quad\land purpose(e)=wait\_resolution\\
\quad\land subject(e)=b\\
\quad\land context(e)=context(b)\\
\quad\land claims(e)=\varnothing,
&subject(e)\in\mathcal B.
\end{cases}
\]

\(EvidenceAdmission_q(e)\) 只在 Evidence 首次进入 Trail 的前态 \(q\) 上求值；它不是要求历史 Evidence 的 subject 永久保持 current 的持续状态谓词。

因此接纳是 main Agent 的显式翻译与一次原子状态转移，而不是对模型外结果的引用升级。任何字段缺失、subject 非 current、Context 过期、payload 未冻结，或在依赖外部 artifact 时无法区分被观察版本的候选输入，都不能进入 \(\Omega\)。它可以留在模型外供后续调查使用，但不具有当前 Evidence 地位。

---

## 6. ReviewPacket 与 ReviewDecision

### 6.1 完整证据宇宙

令 \(q\) 为准备 Seal Review 的前态。先从该配置的 \(\Omega\) 定义 typed Stage Evidence 子集：

\[
\mathcal E_q^{stage}:=
\{e\in\Omega(q)\cap\mathcal E\mid
purpose(e)=stage\_review
\land subject(e)\in\mathcal A\}.
\]

在该子集上定义 \(attempt(e):=subject(e)\)。当前 Stage \(s\)、Context \(\chi\) 的完整 Stage Evidence 宇宙为：

\[
U_q(s,\chi)=
\{e\in\mathcal E_q^{stage}\mid
stage(route(attempt(e)))=s
\land context(e)=\chi
\}.
\]

因此 \(U_q\) 是一个由当前显式配置唯一确定的有限集合。

Evidence 在进入 Trail 时已经满足归属与 Context 条件。旧 Context Evidence 保留历史查询能力；当前 Packet 排除该集合。

### 6.2 ReviewPacket

ReviewPacket 是一次冻结的复核输入：

\[
P=(attempt,stage,context,termination,evidenceSet)
\in\mathcal P.
\]

它必须满足：

\[
\begin{aligned}
P.attempt&=a,\\
P.stage&=stage(route(a)),\\
P.context&=context(a),\\
P.termination&\in\{submitted,bound\_reached,interrupted\},\\
P.evidenceSet&\ne\varnothing,\\
&\exists e\in P.evidenceSet:\ subject(e)=a,\\
P.evidenceSet&=U_q(P.stage,P.context).
\end{aligned}
\]

令 \(DirectProofs(P)\) 为 \(P.context\) 第二个分量中的直接 dependency Decisions。Packet 的完整依赖 proof 视图递归定义为：

\[
\begin{aligned}
DependencyView(P):={}&DirectProofs(P)\\
&\cup
\bigcup_{D\in DirectProofs(P)}
DependencyView(packet(D)).
\end{aligned}
\]

Direct dependency edges 遵循 Stage DAG，且 Packet 引用的 dependency Decision 在 Trail 中早于当前 Packet 被接纳。这两项约束共同保证递归良定并终止。final integration Stage 复核时，main Agent 接收完整 \(DependencyView(P)\)。该视图由冻结 Context 唯一派生，模型保留单一派生路径。Packet 的 Evidence 已逐项满足 \(FrozenEvidence\)，因此 Packet 冻结的是确定的观察值与版本身份，而不是一组需要复读可变 locator 才能确定内容的指针。Packet 一旦 Seal 就保持不变。Seal 在 Trail 中的位置由事实序号派生。新的 Stage Evidence 进入新的 Packet。

同一 Packet 可以完整包含多个材料 baseline 的 Evidence，因为 \(U_q\) 仍按 Stage 与 Acceptance Context 定义，
不能因后续材料漂移而删减。main Agent 必须读取全部 Evidence，再按当前开放世界事实分类 applicability；
`superseded` 解释历史变化但不能单独支撑当前 `satisfied`，`unverifiable` 不能支撑确定性判断。依赖可变材料的
Criterion 至少需要一条覆盖它的 `current_applicable` 且 `supports` 的 Evidence；当前 `contradicts` 与 `unknown`
必须在 findings 中得到处置。这里是正式 Reviewer 的语义责任，不增加 Programmatic accept guard 的外部世界输入。

### 6.3 ReviewDecision

进入 `Reviewing` 后，符合 Mobius 心智模型的 Stage Review 顺序固定为：

1. main Agent 递归闭合 Packet、依赖 proof、Evidence、artifact 与 applicability，并冻结本次 Review 材料；
2. 为该冻结版本创建一个新的 required Judge task，固定 questions、criteria、known risks 与 required coverage；
3. main Agent 检查 native final result、完整通用 envelope、freeze match、coverage 与 Judge findings；
4. main Agent 独立完成语义复核并构造唯一 ReviewDecision。

每个 Review attempt 恰有一个当前 required Judge slot；材料、Packet、baseline 或 Review 问题变化后，旧结果失去
当前资格并创建新任务。额外 Judge 只在覆盖不同问题或反证面时使用。required Judge 缺席、不可用、超时、输出
无效、stale、partial 或 inconclusive 时，`accept` 路径停止；main Agent 可以保持 `Reviewing`，或在语义上成立时
选择模型允许的 non-accept outcome。有效 Judge 结果只是 `accept` 的必要非充分条件：main Agent 仍须独立闭合
Evidence 与 applicability，并以可核查依据解决 Judge findings；未解决的 objection 阻止 `accept`。

上述顺序是 Agentic lifecycle contract。Judge task/result 不进入 \(\Omega\)、Trail 或 Core state，Programmatic
guard 因而不伪装成能认证模型外执行；审计必须把 Core/Trail 健康与 Agent-path 是否完成该仪式分开报告。

main Agent 以正式 Reviewer 身份对一个 Packet 产生：

\[
D=(packet,judgments,findings,action)\in\mathcal D,
\]

其中：

\[
judgments_D:C_\mu(P.stage)
\to\{satisfied,not\_satisfied,unknown\},
\]

\[
action(D)\in
\{accept,retry,replace,wait(b),remap(reason)\}.
\]

其中 \(b\) 或 \(reason\) 属于相应 action 的语义内容，调用者无法额外补充一份状态输入。main Agent 在裁决前必须消费当前 required Judge advice，也可以消费覆盖不同问题的额外模型外 advisory input；它们都不是 ReviewDecision、不能直接改变状态，也不能替代 Packet 的完整 Evidence。令 \(q\) 表示第 8 节定义的当前配置，Decision 的共同适用条件为：

\[
\begin{aligned}
Applicable_q(D,P,\mu)\iff{}&packet(D)=P\\
&\land ObjectiveState(q)=Navigating(j,M^\mu,Reviewing(s,r,a,P))\\
&\land attemptState(a)=sealed\\
&\land P.stage=s
\land P.context=\chi_{\mu,q}(s)\\
&\land domain(judgments_D)=C_\mu(s).
\end{aligned}
\]

`accept` 的必要条件为：

\[
action(D)=accept
\Rightarrow
\forall c\in C_\mu(P.stage):
judgments_D(c)=satisfied.
\]

其余 action 的含义为：

| Action | 语义结果 |
|---|---|
| `retry` | 核心 Route 假设仍成立；Agent 修复后在同一 Route 上开始新 Attempt。 |
| `replace` | 当前 Route 假设不再采用；Stage 返回寻路状态。 |
| `wait(b)` | 存在明确的外部等待条件；Stage 暂停，Objective 继续 active。 |
| `remap(reason)` | 当前 Stage 拆分、依赖或验收认知已不再可信；返回 Mapping。 |

main Agent 提供最终语义判断。状态转移系统决定判断的适用性与应用后的唯一状态。

`Block` 归入 findings 对受阻原因的描述，实体集与终态集保持原值：Route 假设被证伪时选择 `replace`，外部条件暂不可用时选择 `wait`，Stage、依赖或验收认知失效时选择 `remap`。这些 action 实现“触发 Block 后退出当前尝试”，并共享 Review outcome 状态机。

---

## 7. WaitCondition

WaitCondition 表达一个可恢复的外部事实：

\[
b=(stage,context,cause,responsibleParty,resumeCondition)
\in\mathcal B.
\]

WaitCondition 由主循环中的 `wait` Decision 创建。它必须说明：

1. 在等待什么；
2. 谁或什么环境能够改变该事实；
3. 什么观察足以请求恢复。

`resumeCondition` 属于语义条件；程序不会执行 Agent 生成的任意谓词。

检查等待条件需要：

\[
CheckWait(b,E_b,J_b),
\]

先定义当前配置中该 WaitCondition 已接纳的累计证据：

\[
W_q(b):=
\{e\in\Omega(q)\cap\mathcal E\mid
subject(e)=b
\land purpose(e)=wait\_resolution
\land context(e)=context(b)\}.
\]

其中有限非空集 \(E_b\subseteq\mathcal E\) 是本次新提交的 `wait_resolution` Evidence，\(J_b\) 是 main Agent 对当前 WaitCondition 与完整累计 Evidence 集的正式判断：

\[
J_b=(waitCondition,evidenceSet,direction,rationale),
\qquad
J_b.direction\in
\{stay,same\_route,new\_route,remap\}.
\]

`stay` 表示条件仍未解除，其余三个值表示条件已经解除并指定恢复方向。每次判断必须满足 \(J_b.evidenceSet=W_q(b)\cup E_b\)；任一 intervening `stay` 都会改变 \(W_q(b)\)，使旧 Judgment 的 guard 在新前态上失败。程序检查当前 \(b\)、Evidence 归属与 Judgment 完整性。人类或其他模型外来源可以提供观察、意图或 advisory input；main Agent 负责把其中适用的内容转义为 \(E_b\) 与 \(J_b\)，并独自承担正式语义判断。

恢复后：

- 若原 Route 仍适用，则回到该 Route 的 Ready 状态；
- 若原 Route 已失效，则回到 SeekingRoute；
- 若等待暴露了 Stage 或依赖认知错误，则 Remap。

WaitCondition 表达可恢复暂停，Objective 继续保持 active。

---

## 8. 组合状态机

完整配置由当前业务状态和已经接纳的不可变知识组成：

\[
q=(ObjectiveState,\Omega,\Lambda),
\]

其中 \(\Omega\) 是截至当前 Trail 位置已经接纳的全部一等对象有限集合；\(\Lambda\) 是同一 Trail 前缀的 lifecycle projection：

\[
\Lambda=(routeStatus,attemptState,InvalidatedProofs).
\]

其中 \(routeStatus\) 与 \(attemptState\) 构成 lifecycle 投影，\(InvalidatedProofs\subseteq\Omega\cap\mathcal D\) 记录在 Remap 中失去当前 proof 资格的 accepted Decisions。\(\Lambda\) 由 Trail reducer 计算；每次转移同时给出 \(\Omega\) 与 \(\Lambda\) 的唯一后态。`routeStatus=rejected` 只从已经应用的 `Decision(D:replace)` 或 `CheckWait(...,J_b)` 且 \(J_b.direction=new\_route\) 事实派生。Decision 在 review 转移实际应用时进入 \(\Omega\)，Decision application status 由此派生。\(KnownRoutes\)、\(Proof_{\mu,q}\) 和完整 Evidence universe 都由当前配置唯一派生。

下文省略未发生变化的分量；相同的可见 NavState 仍可能具有不同的 \(\Omega\)。

### 8.1 ObjectiveState

Mobius 使用一个和类型表达所有可见业务状态：

\[
\begin{aligned}
ObjectiveState::={}&
Idle\\
&\mid Mapping(j,O^\nu,previousMap?,reason?)\\
&\mid Navigating(j,M^\mu,NavState)\\
&\mid Achieved(j,M^\mu,Manifest)\\
&\mid Abandoned(j,reason).
\end{aligned}
\]

不存在独立的 `finalizing`、`blocked terminal` 或 `Candidate pending` 状态。

### 8.2 NavState

严格串行由 NavState 的结构直接保证：

\[
\begin{aligned}
NavState::={}&
SeekingRoute(s)\\
&\mid Ready(s,r)\\
&\mid Attempting(s,r,a)\\
&\mid Reviewing(s,r,a,P)\\
&\mid Waiting(s,r,b).
\end{aligned}
\]

每个 NavState 恰好包含一个 Stage，串行 cursor 已由类型结构唯一确定。

### 8.3 Stage 派生状态

给定当前 Map 和 ObjectiveState：

\[
stageState_\mu(s)\in\{queued,current,achieved,retired\}.
\]

- \(Proof_{\mu,q}(s)\) 有定义时为 `achieved`；
- \(s\) 出现在当前 NavState 中时为 `current`；
- 属于当前 Map、尚无 proof 且未出现在 current NavState 时为 `queued`；
- 不属于当前 Map 的历史 Stage 为 `retired`。

Stage 状态从 proof 与 current cursor 派生，事实源保持唯一。

---

## 9. 调度

已达成 Stage 集为：

\[
A_{\mu,q}:=\{s\in S_\mu\mid Proof_{\mu,q}(s)\text{ 有定义}\}.
\]

可调度集合为：

\[
ReadyStages_{\mu,q}:=
\{s\in S_\mu\setminus A_{\mu,q}
\mid Dep_\mu(s)\subseteq A_{\mu,q}\}.
\]

下一 Stage 唯一确定为：

\[
next_{\mu,q}=
\arg\min_{s\in ReadyStages_{\mu,q}}(\pi_\mu(s),s),
\]

其中 Stage identity 上存在稳定全序。

因为 \(G_\mu\) 是有限 DAG，且 proof carry 保持已达成集合对依赖关系向下闭合，所以：

\[
A_{\mu,q}\ne S_\mu
\Rightarrow ReadyStages_{\mu,q}\ne\varnothing.
\]

这保证 Remap 或 Stage acceptance 后的未完成状态始终具有可调度 Stage。

---

## 10. 原子状态转移

记：

\[
q\xrightarrow{x}q'
\]

表示状态 \(q\) 在输入 \(x\) 满足前提时，原子转移为 \(q'\)，并向 Trail 追加一个对应事实。前提不成立时没有状态变化。

记 \(q_{active}\) 表示 ObjectiveState 为 Mapping 或 Navigating 的配置；Idle 与两个终态都不属于 \(q_{active}\)。

### 10.1 Objective activation

\[
Idle\xrightarrow{ActivateObjective(O_j^\nu,H^{confirm})}
Mapping(j,O_j^\nu,\bot,initial).
\]

前提是 \(H^{confirm}\) 明确绑定并确认 \(O_j^\nu\)、\(C^\nu\ne\varnothing\)，且：

\[
FreshBatch_\Omega(\{j,O_j^\nu\}\cup C^\nu).
\]
该转移把 \(\{j,O_j^\nu\}\cup C^\nu\) 加入初始 \(\Omega\)。

### 10.2 Map installation

\[
Mapping(j,O^\nu,\_,\_)
\xrightarrow{InstallMap(M^\mu,R^0,J^{cover},J^{carry})}
\begin{cases}
Achieved(j,M^\mu,Manifest_{\mu,q'}),& Complete_{\mu,q'}(j),\\
Navigating(j,M^\mu,SeekingRoute(next_{\mu,q'})),& \text{otherwise}.
\end{cases}
\]

安装时原子验证 Map，并要求 \(J^{cover}\) 的 Map、ObjectiveSpec 与当前 revision 精确一致且 verdict 为 `covered`。令：

\[
X_{install}:=
\{M^\mu\}\cup S_\mu\cup\widehat C_\mu\cup R^0,
\]

其中 \(R^0\) 有限。转移接纳 \(X_{install}\setminus\Omega\)，其余元素按原 identity 引用。若 Mapping 含 previous Map \(M^{\bar\mu}\)，\(J^{carry}\) 的定义域必须精确等于 \(EligibleStages_q(\bar\mu,\mu)\)；首次安装时该定义域为空。程序按拓扑序计算 carry。对于含 previous Map 的安装，后态满足：

\[
\begin{aligned}
InvalidatedProofs(q')={}&InvalidatedProofs(q)\\
&\cup\{Proof_{\bar\mu,q}(s)\mid
s\in S_{\bar\mu}
\land Proof_{\bar\mu,q}(s)\text{ 有定义}
\land \neg Carry_q(s,\bar\mu,\mu)\}.
\end{aligned}
\]

该方程使 invalidation 单调累积，carry 成立的 Decision 保持有效。首次安装保留 \(InvalidatedProofs(q')=InvalidatedProofs(q)=\varnothing\)。每个 \(r\in R^0\) 必须满足当前 Structural Context，新增 Route 的初始 status 为 `available`。

转移把新对象加入 \(\Omega\)，形成完整后态 \(q'\)，再以 \(Complete_{\mu,q'}\) 唯一选择完成或调度分支。final integration proof-tree 的一致性由 \(TreeCompatible\) 在同一 carry 递归中验证。\(KnownRoutes\) 由当前配置派生。

### 10.3 Route enrichment

main Agent 为当前 Stage 提出 Route 后，若 \(Fresh_\Omega(r)\)，且其 Stage 与当前 Map 的 Structural Context 匹配，则：

\[
SeekingRoute(s)
\xrightarrow{AddRoute(r)}
SeekingRoute(s),
\]

其中 \(stage(r)=s\) 且 \(structuralContext(r)=\kappa_\mu(s)\)。该转移增长 Map 知识；MapRevision 与 current Stage 保持原值；\(r\) 进入 \(\Omega\)，初始 status 为 `available`。

选择 Route：

\[
SeekingRoute(s)
\xrightarrow{SelectRoute(r)}
Ready(s,r),
\qquad r\in KnownRoutes_\Omega(s)
\land routeStatus(r)=available
\land structuralContext(r)=\kappa_\mu(s).
\]

当前选择关系由 \(Ready(s,r)\) 独占表达；Route 本身及其 status 保持原值。

### 10.4 Attempt cycle

开始 Attempt：

\[
Ready(s,r)
\xrightarrow{StartAttempt(a)}
Attempting(s,r,a).
\]

该转移要求 \(routeStatus(r)=available\)、\(route(a)=r\) 且 \(context(a)=\chi_{\mu,q}(s)\)，把 \(a\) 加入 \(\Omega\)，并令 \(attemptState(a)=running\)。还必须满足：

\[
\begin{aligned}
Fresh_\Omega(a),
\qquad
a.ordinal=1+\max\big(
\{a'.ordinal\mid a'\in\Omega\cap\mathcal A
\land route(a')=r\}\cup\{0\}
\big).
\end{aligned}
\]

记录 Evidence：

\[
Attempting(s,r,a)
\xrightarrow{RecordEvidence(e)}
Attempting(s,r,a).
\]

该转移要求 \(EvidenceAdmission_q(e)\)。满足该谓词时，\(e\) 已经是由 main Agent 构造的完整、冻结且绑定当前 Attempt 的模型对象；转移只负责原子接纳并把 \(e\) 加入 \(\Omega\)。

Seal 并冻结完整 Packet：

\[
Attempting(s,r,a)
\xrightarrow{SealAttempt(P,sealReason)}
Reviewing(s,r,a,P).
\]

`SealAttempt` 适用于主动提交、达到 bound 或被中断三种情况。它要求 \(sealReason\in\{submitted,bound\_reached,interrupted\}\)、\(attemptState(a)=running\)、\(Fresh_\Omega(P)\)、\(P.termination=sealReason\)，且本次 Attempt 至少贡献一条 Evidence；随后把 \(a\) 投影为 `sealed`，并把满足 \(P.evidenceSet=U_q(P.stage,P.context)\) 的 \(P\) 加入 \(\Omega\)。调用者无法通过选择旧前缀遗漏已经接纳的 Evidence，main Agent 也会看到冻结的终止类别。Packet 保持不可变。

### 10.5 Review outcomes

接受 Stage：

\[
Reviewing(s,r,a,P)
\xrightarrow{Decision(D:accept)}
\begin{cases}
Achieved(j,M^\mu,Manifest_{\mu,q'}),& Complete_{\mu,q'}(j),\\
Navigating(j,M^\mu,SeekingRoute(next_{\mu,q'})),& \text{otherwise}.
\end{cases}
\]

所有 Review outcome 都要求 \(Applicable_q(D,P,\mu)\)。除 `wait(b)` 外要求 \(Fresh_\Omega(D)\)；`wait(b)` 分支要求 \(FreshBatch_\Omega(\{D,b\})\)。转移把 \(D\) 加入 \(\Omega\)，并令 \(attemptState(a)=closed\)、\(closeReason(a)=reviewed\)。accept 转移使 \(D\) 在结果配置 \(q'\) 中满足 \(Proof_{\mu,q'}(s)\) 的派生条件，随后判断 \(Complete_{\mu,q'}(j)\)。Stage acceptance 与 Objective achievement 合并为同一个理论转移，中间 Candidate 已被删除。

accept、retry、wait 与 remap 保持 Route status；`Decision(D:replace)` 是两类合法 route-invalidating Judgment 之一，并将当前 Route 投影为 `rejected`。被证伪的 Route 使用 `replace`；`wait` 表示外部条件尚未满足。所有分支都令该 Attempt 为 `closed`，ordinal 严格递增规则为新尝试生成新 Attempt。

同 Route 修复：

\[
Reviewing(s,r,a,P)
\xrightarrow{Decision(D:retry)}
Ready(s,r).
\]

替换 Route：

\[
Reviewing(s,r,a,P)
\xrightarrow{Decision(D:replace)}
SeekingRoute(s).
\]

进入等待：

\[
Reviewing(s,r,a,P)
\xrightarrow{Decision(D:wait(b))}
Waiting(s,r,b).
\]

该分支还要求 \(stage(b)=s\) 与 \(context(b)=\chi_{\mu,q}(s)\)，并把由 \(D\) 描述的 \(b\) 加入 \(\Omega\)。

请求 Remap：

\[
Reviewing(s,r,a,P)
\xrightarrow{Decision(D:remap(reason))}
Mapping(j,O^\nu,M^\mu,reason).
\]

### 10.6 Check wait

\[
Waiting(s,r,b)
\xrightarrow{CheckWait(b,E_b,J_b)}
\begin{cases}
Waiting(s,r,b),& J_b.direction=stay,\\
Ready(s,r),& J_b.direction=same\_route,\\
SeekingRoute(s),& J_b.direction=new\_route,\\
Mapping(j,O^\nu,M^\mu,wait\_revealed\_drift),& J_b.direction=remap.
\end{cases}
\]

该转移要求 \(E_b\ne\varnothing\)、\(FreshBatch_\Omega(E_b)\)、\(J_b.waitCondition=b\)、\(J_b.evidenceSet=W_q(b)\cup E_b\)，并且 \(\forall e\in E_b:EvidenceAdmission_q(e)\)；随后把 \(E_b\) 加入 \(\Omega\)，并在 Trail 中记录 \(J_b\) 与结果分支。`stay`、`same_route` 与 `remap` 保持 Route status；\(J_b.direction=new\_route\) 是另一类合法 route-invalidating Judgment，并令当前 `Waiting(s,r,b)` 中的 Route \(r\) 为 `rejected`。离开 current Waiting 状态后，旧 WaitCondition 保留历史查询能力。

### 10.7 Remap from any navigation point

客观事实可能在 Attempt 或 Review 之外暴露 Map 错误。因此任一 Navigating 状态都允许经明确判断请求 Remap：

\[
Navigating(j,M^\mu,nav)
\xrightarrow{RequestRemap(reason)}
Mapping(j,O^\nu,M^\mu,reason).
\]

若当前 NavState 含有 running 或 sealed Attempt，该转移将其投影为 `closed`，原因 `remapped`。Packet 与 WaitCondition 随 NavState 退出 current 范围，所有对象继续保留在 \(\Omega\) 与 Trail 中。Route status 保持原值，后续可选择性由新 Map 的 Structural Context 决定。新 Map 的 proof 继承范围限定为结构 eligible 且语义判断仍 valid 的集合。

main Agent 每次造成材料 effect 后还要核对它对全部已接受 proof 的影响。只影响 current Stage 的变化留在当前
Attempt 并形成新 Evidence；影响已接受 Stage 的材料、工具链、共享配置或安全边界，或影响未知时，从当前
Navigating 状态提交 `RequestRemap`，在新 Map 中对受影响 Stage 及传递依赖给出 `invalid` carry 并重新验收。
Stage contract、Criterion ownership 或依赖认知变化时同样 Remap 并修订 Map。能够证明无范围/依赖影响时继续，
不制造空 Remap。该 proof-impact 判断是 ephemeral Agentic 分析，不是新对象或 transition。

### 10.8 ObjectiveSpec revision

ObjectiveSpec revision 是带人类授权的 Remap：

\[
q_{active}
\xrightarrow{ReviseObjective(O_j^{\nu'},H^{confirm})}
Mapping(j,O_j^{\nu'},previousMap(q),spec\_revised).
\]

它要求 \(H^{confirm}\) 明确绑定并确认 \(O_j^{\nu'}\)，且 \(Fresh_\Omega(O_j^{\nu'})\)。稳定 Objective identity \(j\) 保持不变；转移把 \(\{O_j^{\nu'}\}\cup C^{\nu'}\) 中尚未接纳的对象加入 \(\Omega\)。若 revision 从 Navigating 发起，它与 `RequestRemap` 一样关闭当前未完成的 navigation context；历史对象仍保留在 Trail 中。

### 10.9 Abandon

任一非终态 Objective 都可以由人类放弃：

\[
q_{active}
\xrightarrow{Abandon(reason,H^{abandon})}
Abandoned(j,reason).
\]

\(H^{abandon}=(j,reason,confirmed)\) 必须来自人类对完整放弃 action 的明确确认，并精确绑定当前 Objective 与放弃原因；它与 \(H^{confirm}\) 使用相同的语义 ownership，不要求 Runtime 身份证明。该转移将 current running 或 sealed Attempt 投影为 `closed`、原因 `abandoned`；其余 navigation 对象随 NavState 一同退出 current 范围。`Achieved` 与 `Abandoned` 都是终态，后续业务状态转移全部拒绝。

---

## 11. Objective 完成

定义：

\[
AllCurrent_{\mu,q}
\iff
\forall s\in S_\mu:\ Proof_{\mu,q}(s)\text{ 有定义}.
\]

完成条件为：

\[
Complete_{\mu,q}(j)
\iff
AllCurrent_{\mu,q}.
\]

若 final integration Stage \(s_\star\) 存在，\(Proof_{\mu,q}(s_\star)\) 的当前性已经要求其直接 dependency proofs 与 \(\chi_{\mu,q}(s_\star)\) 一致；沿 DAG 递归展开即可得到全部 transitive current proofs。因而 \(DependencyView\) 的精确性属于 \(Proof\) 当前性的推论，完成条件保持一个。

因为 \(C^\nu\subseteq\widehat C_\mu\)、\(owner_\mu:\widehat C_\mu\to S_\mu\) 是全函数，且每个 accepted Decision 都满足其 Stage 的全部 Criterion，所以：

\[
Complete_{\mu,q}(j)
\Rightarrow
\forall c\in C^\nu:\ c\text{ 已被其 owner Stage 的当前 proof 接受}.
\]

最终 manifest 包含完成事实中的精确 proof 集：

\[
Manifest_{\mu,q}=
\{(s,Proof_{\mu,q}(s))\mid s\in S_\mu\}.
\]

Manifest 随 `Achieved` 事实一次形成，没有独立生命周期。

`Achieved` 与 `Abandoned` 形成后，开放世界中的后续漂移或缺陷不重开、Remap 或改写该终态。它们只能成为
审计 finding；需要继续改变世界时，由人类显式授权一个新的 Objective。

---

## 12. Trail 与重放

Trail 是状态转移事实序列：

\[
Trail_j=(f_1,f_2,\ldots,f_n).
\]

每个事实记录：

\[
f_i=(objective,transition,input).
\]

reason、rationale、sealReason 与各类 Judgment 集中在相应 transition input 中；Trail 保留单一解释来源。
Route rejection 不产生独立事实或附加 payload：它只由已应用的 `Decision(D:replace)`，或前态为
`Waiting(s,r,b)` 且 \(J_b.direction=new\_route\) 的 `CheckWait(b,E_b,J_b)` 事实派生。无法追溯到这两类
transition input 的 `routeStatus=rejected` 投影违反重放不变量。

初始完整配置为：

\[
q_0=(Idle,\varnothing,(\varnothing,\varnothing,\varnothing)).
\]

状态由纯转移函数逐步派生：

\[
q_i=\delta(q_{i-1},f_i),
\qquad
q_n=fold(\delta,q_0,Trail_j).
\]

合法 Trail 必须满足每一步转移在其前态上 enabled：

\[
\forall i\le n:\quad
Enabled(q_{i-1},f_i),
\]

并且每个事实都对应第 10 节某条转移关系。函数 \(\delta\) 同时归约 ObjectiveState、\(\Omega\) 与 \(\Lambda\)，所以对象成员关系、Decision 接纳关系和 lifecycle 都能从同一 Trail 前缀重建。`fromState/toState` 由历史前缀计算，Trail 无需保存副本。

重放不变量为：

\[
Replay(Trail_j)=q_n.
\]

Trail 保存发生了什么、基于什么 Packet 和 Judgment、为何进入下一状态；这些内容都来自唯一的 transition input。它不在数学模型中规定这些事实如何落盘。

---

## 13. 安全不变量

所有可达状态必须满足：

\[
\begin{aligned}
I_1 &: \text{一个 Objective 至多有一个 current Stage},\\
I_2 &: \text{NavState 恰属于一个 variant，且其中每一类 current 对象至多一个},\\
I_3 &: \operatorname{Acyclic}(G_\mu),\\
I_4 &: owner_\mu:\widehat C_\mu\to S_\mu\text{ 是全函数},\\
I_5 &: \text{current Stage 的所有依赖都有当前 proof},\\
I_6 &: \forall e\in\Omega(q_n)\cap\mathcal E,\ \exists i\le n:\\
&\qquad EvidenceAdmission_{q_{i-1}}(e)
\land e\in\Omega(q_i)\setminus\Omega(q_{i-1}),\\
I_7 &: \text{ReviewPacket 冻结 Seal termination 与完整 Evidence 宇宙},\\
I_8 &: accept\text{ 必须判断该 Stage 的全部 Criterion satisfied},\\
I_9 &: \text{Stage achieved 当且仅当存在当前 accepted Decision},\\
I_{10} &: \text{Remap carry 范围限定为结构 eligible、依赖闭合且被显式判定 valid 的 proof},\\
I_{11} &: \text{final integration 的 DependencyView 精确展开当前 proof tree},\\
I_{12} &: \text{Objective achieved 当且仅当 }Complete_{\mu,q}(j),\\
I_{13} &: \text{WaitCondition 可恢复且不进入终态},\\
I_{14} &: \text{Agentic output 不绕过转移前提直接改变状态},\\
I_{15} &: Replay(Trail_j)=q,\\
I_{16} &: \text{终态拒绝后续业务状态转移},\\
I_{17} &: Abandoned\text{ 必须绑定人类确认},\\
I_{18} &: \text{模型外输出不能直接成为 Evidence、Decision 或状态转移},\\
I_{19} &: routeStatus(r)=rejected\iff\text{当前 Trail 前缀中存在已经应用于 }r\text{ 的}\\
&\qquad Decision(D:replace),\text{ 或前态为 }Waiting(s,r,b)\text{ 且}\\
&\qquad J_b.direction=new\_route\text{ 的 }CheckWait(b,E_b,J_b).
\end{aligned}
\]

---

## 14. 结构闭环与条件活性

### 14.1 每个非终态都有明确出口

| 状态 | 合法出口类别 |
|---|---|
| `Idle` | ActivateObjective |
| `Mapping` | InstallMap、ReviseObjective、Abandon |
| `SeekingRoute` | AddRoute、SelectRoute、RequestRemap、ReviseObjective、Abandon |
| `Ready` | StartAttempt、RequestRemap、ReviseObjective、Abandon |
| `Attempting` | RecordEvidence、SealAttempt、RequestRemap、ReviseObjective、Abandon |
| `Reviewing` | accept、retry、replace、wait、remap、ReviseObjective、Abandon |
| `Waiting` | CheckWait、RequestRemap、ReviseObjective、Abandon |

因此每个内部业务状态都有名称和出口。环境未提供所需输入时可以停留，停留原因通过 Mapping、Seeking、Reviewing 或 Waiting 保持可观察。

### 14.2 条件活性

开放世界可能缺少可行 Route，人类、main Agent、模型外工作来源、工具或外部环境也可能中断响应。

这里的公平响应表示：至少一个能够离开当前状态的合法输入最终到达，且 enabled 转移不会被永久忽略。在此前提下：

\[
\begin{aligned}
Ready(s,r)&\Rightarrow\Diamond(Attempting\lor Mapping\lor Abandoned),\\
Attempting(s,r,a)&\Rightarrow\Diamond(Reviewing\lor Mapping\lor Abandoned),\\
Reviewing(s,r,a,P)&\Rightarrow\Diamond(Ready\lor SeekingRoute\lor Waiting\\
&\hspace{7.9em}\lor Mapping\lor Achieved\lor Abandoned),\\
Waiting(s,r,b)\land Resolvable(b)&\Rightarrow\Diamond\neg Waiting(s,r,b).
\end{aligned}
\]

其中 \(Resolvable(b)\) 表示环境最终能够提供一个满足 CheckWait guard、且 direction 不为 `stay` 的 \((E_b,J_b)\)；它属于开放世界假设，程序不从 WaitCondition 文本计算该值。

`SeekingRoute` 与不可解除的 `Waiting` 可以持续停留；下一输入的责任归属保持可观察。

进一步假设：

1. ObjectiveSpec 最终稳定，Remap 次数有限，并且一个有限 DAG Map 最终完成安装；
2. 每个未完成 Stage 都存在可行 Route，且它最终被选择；
3. 在有限次 retry 或 replace 后，某次 Attempt 产生充分 Evidence；
4. main Agent 最终接受充分且完整的 Evidence；
5. enabled 的 progress 转移不会被永久忽略；

则最终稳定 Map 中的每个 Stage 依次取得 proof。Objective 未被放弃时：

\[
\Diamond Achieved(j).
\]

该结论适用于上述条件。任意 Objective 的必然完成超出声明范围。

---

## 15. Programmatic / Agentic 分界

### 15.1 Actor ownership

main Agent 是唯一的 Mobius 编排、事实接纳和正式裁决主体。它拥有当前 Objective、Map、Attempt、Trail
投影、状态转移意图与全部 Agentic input，并负责把开放世界中的工作结果转换成满足 guard 的 Evidence 或
Judgment。对于由同一接纳前态和 Agent intent 唯一决定的机械派生组件，Programmatic boundary 可以在 reducer
之前物化精确 transition input；例如 main Agent 请求 Seal current Attempt 后，程序从完整 Evidence universe
唯一物化 ReviewPacket。该物化不产生第二个事实接纳者、语义裁决者或历史来源。

这里的 main Agent 是语义 owner，而不是本理论模型认证的 Runtime principal。模型假定 Composition、Skill 与
参与工作的 Agent 遵守该 ownership；Programmatic guard 验证 transition input 与当前状态，不验证调用线程或
role identity。恶意、被攻陷或故意违反委托边界的 Agent 不属于本模型的威胁范围。

开放世界工作如何被拆分、委托、执行或审查不属于 Core 对象模型。模型不要求这些来源理解 Objective、Map、
Stage、Attempt、Evidence、ReviewDecision 或 Trail，也不赋予它们任何 Mobius actor 身份。它们可以产生候选
观察、影响声明、产物或 advisory input；Composition 使用通用 Judge task/result 合同完成每个 Stage Review 的
固定仪式，main Agent 独自解释其输出，并构造本模型定义的完整对象。

外部改变在发生时已经是世界事实，但它不会因此自动成为 Mobius 事实。main Agent 必须核查其内容、影响与
provenance，固定 observation 与被观察版本，并提交满足 \(EvidenceAdmission_q\) 的 Evidence。模型外 advisory
input 同样不能直接成为 ReviewDecision；main Agent 必须自行检查 Packet 完整性与反证，再以正式 Reviewer
身份给出唯一 Decision。

模型外工作可以并发，Mobius 状态接纳仍严格串行。工作期间 current subject 或 Acceptance Context 已变化时，
main Agent 不能把旧输出接纳到新 Context，只能将其留在模型外作为线索，或重新获取适用于当前 Context 的观察。

### 15.2 Guard boundary

Programmatic transition guard 具有显式类型域，读取范围限定为当前配置与本次输入：

\[
Programmatic(g)
\iff
\Big(
g:\mathcal Q_g\times\mathcal X_g\to\{true,false\}
\land FV(g)\subseteq\{q,input\}
\Big).
\]

其中 \(\mathcal Q_g\) 与 \(\mathcal X_g\) 必须显式定义，\(FV(g)\) 表示 guard 的自由输入。同一 \(q\) 与 input 总会产生相同结果，并支持 Trail 重放。

### 15.3 Programmatic

- DAG 与 Criterion ownership 校验；
- 严格串行调度；
- Route、Attempt、Evidence、Packet 的归属检查；
- \(EvidenceAdmission\) 的 typed subject、Context、claims domain 与冻结字段检查；
- 完整 Evidence universe 的冻结；
- stale Context 拒绝；
- action 对应的唯一状态转移；
- Remap carry 的结构资格与依赖闭包检查；
- completion manifest；
- Trail reducer 与不变量检查。

### 15.4 Agentic

- 理解和操作化人类意图；
- 拆分或重绘 Stage；
- 设计、修复和替换 Route；
- 把模型外候选观察转义为完整 Evidence，并判断其 observation 与 provenance 是否足以支持声称；
- 捕获依赖可变材料的稳定 baseline，判断验证前后 coherence，并在 Seal、Review 与后续 effect 后重新核对
  applicability 与跨 Stage proof impact；
- 解释 Evidence；
- 在每个 Stage Review 的 Packet closure 与材料冻结后创建并消费一个新鲜 required Judge task，检查其完整性、
  freeze、coverage 与 findings，同时保持正式 Judgment ownership；
- 判断 Criterion 是否满足；
- 判断等待条件是否真实、是否解除；
- 判断结构 eligible 的旧 proof 在新 Map 下是否仍然语义有效；
- 发现 Map 与客观事实之间的冲突；
- 请求并记录人类对 ObjectiveSpec revision 或放弃 Objective 的确认。

程序证明范围：状态转移合法、main Agent 的复核输入完整、Judgment 被忠实记录、完成条件没有遗漏。

开放世界保留：main Agent 或模型外工作来源可能找不到最佳 Route，main Agent 的语义判断可能偏离客观事实，成功所需条件也可能长期缺失。

---

## 16. 模型边界

本理论模型刻意不定义：

- 持久化介质与数据库结构；
- 文件、表、消息或对象格式；
- command、CLI、MCP 或 hook 接口；
- 模型外工作与 advisory input 的调用、重试和关闭 transport；
- 幂等 receipt、锁与并发控制；
- actor 身份、授权协议与部署信任边界；
- 字节规范化、逐实体 hash 和碰撞防御；
- 本机攻击者或分布式系统威胁模型。

这些实现规格用于保护本模型的不变量。Mobius 心智模型保持上述理论边界。
实现可以采用协作式 Agent 信任边界：由 Skill 与 Composition 约束谁应提交 transition，同时由 Core guard
保证提交内容合法；模型不要求 per-thread capability、caller attestation 或 main/subagent 机械隔离。
