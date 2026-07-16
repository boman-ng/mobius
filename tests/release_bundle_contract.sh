#!/usr/bin/env bash
set -euo pipefail

readonly MINIMUM_CODEX_CLI_VERSION=0.143.0

fail() {
  echo "release bundle contract: $*" >&2
  exit 1
}

require_file() {
  [ -f "$1" ] || fail "missing file: $1"
}

stable_semver_at_least() {
  local actual=$1
  local minimum=$2
  local actual_major actual_minor actual_patch
  local minimum_major minimum_minor minimum_patch

  if [[ $actual =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]]; then
    actual_major=$((10#${BASH_REMATCH[1]}))
    actual_minor=$((10#${BASH_REMATCH[2]}))
    actual_patch=$((10#${BASH_REMATCH[3]}))
  else
    return 1
  fi
  if [[ $minimum =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]]; then
    minimum_major=$((10#${BASH_REMATCH[1]}))
    minimum_minor=$((10#${BASH_REMATCH[2]}))
    minimum_patch=$((10#${BASH_REMATCH[3]}))
  else
    return 1
  fi

  ((
    actual_major > minimum_major ||
      (actual_major == minimum_major && actual_minor > minimum_minor) ||
      (actual_major == minimum_major && actual_minor == minimum_minor && actual_patch >= minimum_patch)
  ))
}

validate_codex_version_policy() {
  local supported unsupported

  for supported in 0.143.0 0.143.1 0.144.0 1.0.0; do
    stable_semver_at_least "$supported" "$MINIMUM_CODEX_CLI_VERSION" \
      || fail "Codex minimum-version comparator rejected supported boundary: $supported"
  done
  for unsupported in 0.142.999 0.143 0.143.0-beta.1 malformed; do
    if stable_semver_at_least "$unsupported" "$MINIMUM_CODEX_CLI_VERSION"; then
      fail "Codex minimum-version comparator accepted unsupported boundary: $unsupported"
    fi
  done
}

validate_config() {
  plugin_root=$1
  manifest="$plugin_root/.codex-plugin/plugin.json"
  mcp="$plugin_root/.mcp.json"
  hooks="$plugin_root/hooks/hooks.json"

  require_file "$manifest"
  require_file "$mcp"
  require_file "$hooks"

  jq -e '
    .name == "mobius" and
    (.version | type == "string" and length > 0) and
    .skills == "./skills/" and
    .mcpServers == "./.mcp.json" and
    .hooks == "./hooks/hooks.json"
  ' "$manifest" >/dev/null || fail "manifest component paths are not canonical"

  jq -e '
    (keys == ["mobius"]) and
    .mobius.command == "./bin/mobius" and
    .mobius.args == ["mcp"] and
    .mobius.cwd == "."
  ' "$mcp" >/dev/null || fail "MCP config must expose one server through ./bin/mobius"

  jq -e '
    ([.hooks.PreToolUse[].hooks[].command] | unique) ==
      ["\"${PLUGIN_ROOT}/bin/mobius\" hook pre-tool-use"] and
    ([.hooks.PreToolUse[].matcher] | unique) ==
      ["Bash|Shell|exec_command|write_stdin|apply_patch|Edit|Write|mcp__.*"] and
    ([.hooks.Stop[].hooks[].command] | unique) ==
      ["\"${PLUGIN_ROOT}/bin/mobius\" hook stop"]
  ' "$hooks" >/dev/null \
    || fail "hooks must invoke the packaged binary and cover every protected mutation entrypoint"
}

validate_skill_contracts() {
  plugin_root=$1
  copilot="$plugin_root/skills/mobius-copilot/SKILL.md"
  copilot_interface="$plugin_root/skills/mobius-copilot/agents/openai.yaml"
  loop_interface="$plugin_root/skills/mobius-loop/agents/openai.yaml"
  subagent_interface="$plugin_root/skills/mobius-subagent/agents/openai.yaml"

  require_file "$copilot"
  require_file "$copilot_interface"
  require_file "$loop_interface"
  require_file "$subagent_interface"
  grep -Fqx 'name: mobius-copilot' "$copilot" \
    || fail "Mobius Copilot skill identity is invalid"
  grep -Fq '$mobius-copilot' "$copilot_interface" \
    || fail "Mobius Copilot interface does not invoke the canonical skill"
  for explicit_interface in "$copilot_interface" "$loop_interface"; do
    [ "$(grep -Fc 'allow_implicit_invocation: false' "$explicit_interface")" = "1" ] \
      || fail "Composition skill must disable implicit invocation: $explicit_interface"
    grep -Fq 'value: "mobius"' "$explicit_interface" \
      || fail "Composition skill must declare the Mobius MCP dependency: $explicit_interface"
    grep -Fq 'transport: "stdio"' "$explicit_interface" \
      || fail "Composition skill must declare the bundled stdio transport: $explicit_interface"
  done
  [ "$(grep -Fc 'allow_implicit_invocation: true' "$subagent_interface")" = "1" ] \
    || fail "Mobius Subagent must remain eligible for implicit selection"
  if grep -Fq 'dependencies:' "$subagent_interface"; then
    fail "Mobius Subagent must remain independent of Core MCP dependencies"
  fi
  [ ! -e "$plugin_root/skills/mobius-plan" ] \
    || fail "legacy Mobius Plan skill path must not be packaged"
}

validate_source() {
  repo_root=$1
  source_marketplace="$repo_root/.agents/plugins/marketplace.json"
  source_plugin="$repo_root/plugins/mobius"
  toolchain_file="$repo_root/rust-toolchain.toml"

  require_file "$source_marketplace"
  require_file "$toolchain_file"
  validate_config "$source_plugin"
  validate_skill_contracts "$source_plugin"
  validate_codex_version_policy

  toolchain_version=$(sed -n 's/^channel = "\([^"]*\)"$/\1/p' "$toolchain_file")
  cargo_rust_version=$(sed -n 's/^rust-version = "\([^"]*\)"$/\1/p' \
    "$source_plugin/runtime/Cargo.toml")
  [ "$toolchain_version" = "1.85.0" ] && [ "$cargo_rust_version" = "$toolchain_version" ] \
    || fail "Cargo rust-version and root toolchain must both pin Rust 1.85.0"

  jq -e '
    ([.plugins[] | select(.name == "mobius")] | length) == 1 and
    (.plugins[] | select(.name == "mobius") |
      .source == {"source":"local","path":"./plugins/mobius"} and
      .policy.installation == "NOT_AVAILABLE" and
      .policy.authentication == "ON_INSTALL")
  ' "$source_marketplace" >/dev/null || fail "source marketplace must remain unavailable"

  [ ! -e "$source_plugin/bin/mobius" ] || fail "source tree must not contain a release binary"

  legacy_runtime_path=$(find "$source_plugin" -type f \
    \( -name '*.py' -o -name '*.pyc' -o -name 'uv.lock' \
      -o -name 'mobius_hook_launcher.sh' -o -name 'mobius_review_mcp_server.sh' \) \
    -print -quit)
  if [ -n "$legacy_runtime_path" ]; then
    fail "plugin source contains a Python, uv, or legacy launcher runtime path: $legacy_runtime_path"
  fi

  metadata=$(cargo metadata \
    --manifest-path "$source_plugin/runtime/Cargo.toml" \
    --locked \
    --no-deps \
    --format-version 1)
  jq -e '
    [.packages[] | select(.name == "mobius") | .targets[] |
      select(.kind == ["bin"]) | .name] == ["mobius"]
  ' <<<"$metadata" >/dev/null || fail "Cargo package must expose exactly one mobius binary target"

  manifest_version=$(jq -r '.version' "$source_plugin/.codex-plugin/plugin.json")
  cargo_version=$(jq -r '.packages[] | select(.name == "mobius") | .version' <<<"$metadata")
  [ "$manifest_version" = "$cargo_version" ] \
    || fail "manifest and Cargo versions differ: $manifest_version != $cargo_version"
  changelog_version_pattern=${manifest_version//./\\.}
  grep -Eq "^## $changelog_version_pattern - (Unreleased|[0-9]{4}-[0-9]{2}-[0-9]{2})$" \
    "$repo_root/CHANGELOG.md" \
    || fail "changelog has no exact current section for $manifest_version"
}

validate_bundle_shape() {
  bundle_root=$1
  plugin_root="$bundle_root/plugins/mobius"
  marketplace="$bundle_root/.agents/plugins/marketplace.json"
  binary="$plugin_root/bin/mobius"

  validate_config "$plugin_root"
  validate_skill_contracts "$plugin_root"
  require_file "$marketplace"
  require_file "$bundle_root/LICENSE"
  require_file "$bundle_root/SHA256SUMS"

  jq -e '
    ([.plugins[] | select(.name == "mobius")] | length) == 1 and
    (.plugins[] | select(.name == "mobius") |
      .source == {"source":"local","path":"./plugins/mobius"} and
      .policy.installation == "AVAILABLE" and
      .policy.authentication == "ON_INSTALL")
  ' "$marketplace" >/dev/null || fail "assembled marketplace is not releasable"

  [ ! -d "$plugin_root/runtime" ] || fail "Rust source must not enter the installed plugin"
  legacy_runtime_path=$(find "$plugin_root" -type f \
    \( -name '*.py' -o -name '*.pyc' -o -name 'uv.lock' \
      -o -name 'mobius_hook_launcher.sh' -o -name 'mobius_review_mcp_server.sh' \) \
    -print -quit)
  [ -z "$legacy_runtime_path" ] \
    || fail "bundle contains a Python, uv, or legacy launcher runtime path: $legacy_runtime_path"
  [ -x "$binary" ] || fail "packaged mobius binary is not executable"

  executable_count=$(find "$plugin_root" -type f -perm /111 | wc -l | tr -d ' ')
  [ "$executable_count" = "1" ] || fail "installed plugin must contain exactly one executable"
  executable_path=$(find "$plugin_root" -type f -perm /111 -print)
  [ "$executable_path" = "$binary" ] || fail "unexpected executable entrypoint: $executable_path"

  elf_header=$(readelf -h "$binary")
  grep -q 'Machine:.*Advanced Micro Devices X86-64' <<<"$elf_header" \
    || fail "binary is not x86_64"
  dynamic_dependencies=$(ldd "$binary" 2>/dev/null || true)
  if grep -Eiq 'python|sqlite' <<<"$dynamic_dependencies"; then
    fail "binary has a runtime dependency on Python or system SQLite"
  fi

  script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
  source_root=$(CDPATH= cd -- "$script_dir/.." && pwd -P)
  binary_strings=$(strings -a "$binary")
  if grep -Fq "$source_root" <<<"$binary_strings"; then
    fail "binary contains the release source root"
  fi
  if grep -Eq '/home/[^/[:space:]]+|/Users/[^/[:space:]]+|/root(/|$)' \
    <<<"$binary_strings"; then
    fail "binary contains a personal build-host path"
  fi

  (
    cd "$bundle_root"
    sha256sum --check --strict SHA256SUMS >/dev/null
  ) || fail "bundle checksum verification failed"
}

smoke_installed_plugin() {
  installed=$1
  isolated_root=$2
  manifest_version=$(jq -r '.version' "$installed/.codex-plugin/plugin.json")
  workspace="$isolated_root/workspace with spaces"
  bound_container="$isolated_root/bound-container"
  project_b="$bound_container/project B with spaces"
  ordinary_cwd="$isolated_root/ordinary-cwd"
  mkdir -p "$workspace/debug/build" "$project_b" "$ordinary_cwd"

  help_output=$(
    cd "$workspace"
    env -i HOME="$isolated_root/home" PATH=/nonexistent "$installed/bin/mobius" --help
  ) || fail "cache-copied binary did not start in an empty environment"
  for mode in mcp read audit doctor report hook; do
    grep -q "mobius $mode" <<<"$help_output" || fail "help omits mode: $mode"
  done

  initialize_request='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"release-smoke","version":"1"}}}'
  initialized_notification='{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}'

  pre_tool_use_probe() {
    local tool_name=$1
    local tool_input=$2
    local hook_cwd=${3:-$workspace}
    local request
    request=$(jq -nc \
      --arg cwd "$hook_cwd" \
      --arg tool_name "$tool_name" \
      --argjson tool_input "$tool_input" \
      '{hook_event_name:"PreToolUse",cwd:$cwd,tool_name:$tool_name,tool_input:$tool_input}')
    (
      cd "$workspace"
      printf '%s\n' "$request" \
        | env -i HOME="$isolated_root/home" PATH=/nonexistent \
          "$installed/bin/mobius" hook pre-tool-use
    ) || fail "installed pre-tool-use hook failed for $tool_name"
  }

  expect_hook_deny() {
    local description=$1
    local tool_name=$2
    local tool_input=$3
    local hook_cwd=${4:-$workspace}
    local response
    response=$(pre_tool_use_probe "$tool_name" "$tool_input" "$hook_cwd")
    jq -e '
      .hookSpecificOutput.hookEventName == "PreToolUse" and
      .hookSpecificOutput.permissionDecision == "deny"
    ' <<<"$response" >/dev/null || fail "installed hook allowed $description"
  }

  expect_hook_allow() {
    local description=$1
    local tool_name=$2
    local tool_input=$3
    local hook_cwd=${4:-$workspace}
    local response
    response=$(pre_tool_use_probe "$tool_name" "$tool_input" "$hook_cwd")
    [ -z "$response" ] || fail "installed hook denied $description: $response"
  }

  stop_probe() {
    local message=$1
    local request
    request=$(jq -nc \
      --arg cwd "$workspace" \
      --arg message "$message" \
      '{
        hook_event_name: "Stop",
        cwd: $cwd,
        stop_hook_active: false,
        last_assistant_message: $message
      }')
    (
      cd "$workspace"
      printf '%s\n' "$request" \
        | env -i HOME="$isolated_root/home" PATH=/nonexistent \
          "$installed/bin/mobius" hook stop
    ) || fail "installed stop hook failed"
  }

  initialize_project() {
    local root=$1
    local call_id=$2
    local request_id=$3
    local root_uri request response
    root_uri="file://${root// /%20}"
    request=$(jq -nc \
      --arg root "$root" \
      --arg uri "$root_uri" \
      --arg request_id "$request_id" \
      --argjson id "$call_id" '
      {
        jsonrpc: "2.0",
        id: $id,
        method: "tools/call",
        params: {
          name: "mobius_project_init",
          arguments: {
            project_root: $root,
            request_id: $request_id
          },
          _meta: {
            "codex/sandbox-state-meta": {
              sandboxCwd: $uri
            }
          }
        }
      }')
    response=$(
      cd "$root"
      printf '%s\n%s\n%s\n' "$initialize_request" "$initialized_notification" "$request" \
        | env -i HOME="$isolated_root/home" PATH=/nonexistent \
          "$installed/bin/mobius" mcp
    ) || fail "cache-copied MCP server did not initialize $root"
    jq -s -e --argjson id "$call_id" --arg manifest_version "$manifest_version" '
      ([.[] | select(
        .id == 1 and
        .result.serverInfo.name == "mobius" and
        .result.serverInfo.version == $manifest_version
      )] | length) == 1 and
      ([.[] | select(.id == $id and .result.isError == false)] | length) == 1
    ' <<<"$response" >/dev/null \
      || fail "cache-copied MCP version or project_init failed for $root"
  }

  unbound_state_mutation=$(jq -nc \
    --arg cmd "rm -f '$isolated_root/other/.mobius/mobius.sqlite3'" '{cmd:$cmd}')
  expect_hook_deny \
    "an explicit absolute Core-state target before project binding" \
    exec_command \
    "$unbound_state_mutation"
  expect_hook_allow \
    "recursive deletion of an unbound ordinary cwd" \
    exec_command \
    "$(jq -nc --arg cmd "rm -rf '$ordinary_cwd'" '{cmd:$cmd}')" \
    "$ordinary_cwd"

  initialize_project "$workspace" 2 bundle-smoke-project-init
  initialize_project "$project_b" 3 bundle-smoke-project-b-init

  expect_hook_deny \
    "recursive deletion of a bound project root" \
    exec_command \
    "$(jq -nc --arg cmd "rm -rf '$workspace'" '{cmd:$cmd}')"
  expect_hook_allow \
    "recursive deletion confined below a bound project root" \
    exec_command \
    "$(jq -nc --arg cmd "rm -rf '$workspace/debug/build'" '{cmd:$cmd}')"
  expect_hook_allow \
    "a derived view write" \
    mcp__filesystem__write_file \
    "$(jq -nc --arg path "$workspace/.mobius/views/current" '{path:$path,content:"derived"}')"
  expect_hook_deny \
    "project A hook cwd with project B tool workdir" \
    exec_command \
    "$(jq -nc --arg workdir "$project_b" '{cmd:"rm -rf .",workdir:$workdir}')"
  expect_hook_allow \
    "an unbound deterministic read of project B state" \
    exec_command \
    "$(jq -nc --arg cmd "file '$project_b/.mobius/mobius.sqlite3'" '{cmd:$cmd}')" \
    "$ordinary_cwd"

  connector_newline=$(jq -nc --arg cmd "cd '$project_b' &&
find . -delete" '{cmd:$cmd}')
  expect_hook_deny \
    "a connector newline entering project B" \
    exec_command \
    "$connector_newline" \
    "$ordinary_cwd"
  short_circuit=$(jq -nc \
    --arg cmd "cd '$project_b' || cd '$ordinary_cwd' && find . -delete" '{cmd:$cmd}')
  expect_hook_deny \
    "a live project B short-circuit branch" \
    exec_command \
    "$short_circuit" \
    "$ordinary_cwd"
  expect_hook_deny \
    "a structured target ancestor containing project B" \
    mcp__filesystem__delete_directory \
    "$(jq -nc --arg target "$bound_container" '{target:$target,recursive:true}')" \
    "$ordinary_cwd"

  no_claim_response=$(stop_probe "ordinary completion summary")
  [ -z "$no_claim_response" ] || fail "installed stop hook blocked a message without a claim"
  false_claim_response=$(stop_probe "MOBIUS_OBJECTIVE_ACHIEVED: objective-1")
  jq -e '.decision == "block"' <<<"$false_claim_response" >/dev/null \
    || fail "installed stop hook accepted an unachieved Objective claim"

  rm -rf -- "$workspace/.mobius" "$project_b/.mobius"
}

validate_bundle() {
  bundle_root=$1
  plugin_root="$bundle_root/plugins/mobius"
  validate_bundle_shape "$bundle_root"

  cache_root=$(mktemp -d)
  trap 'rm -rf "$cache_root"' EXIT
  version=$(jq -r '.version' "$plugin_root/.codex-plugin/plugin.json")
  installed="$cache_root/home/.codex/plugins/cache/mobius/mobius/$version"
  mkdir -p "$(dirname "$installed")"
  cp -R "$plugin_root" "$installed"
  smoke_installed_plugin "$installed" "$cache_root"
}

validate_codex_install() {
  bundle_root=$1
  validate_bundle_shape "$bundle_root"

  codex_binary=$(command -v codex || true)
  [ -n "$codex_binary" ] || fail "codex-install mode requires the Codex CLI"
  codex_version=$("$codex_binary" --version) \
    || fail "could not read the Codex CLI version"
  case $codex_version in
    "codex-cli "*) actual_codex_version=${codex_version#"codex-cli "} ;;
    *) fail "codex-install mode requires codex-cli >= $MINIMUM_CODEX_CLI_VERSION; found: $codex_version" ;;
  esac
  stable_semver_at_least "$actual_codex_version" "$MINIMUM_CODEX_CLI_VERSION" \
    || fail "codex-install mode requires stable codex-cli >= $MINIMUM_CODEX_CLI_VERSION; found: $codex_version"

  install_parent=$(CDPATH= cd -- "$(dirname -- "$bundle_root")" && pwd)
  install_root=$(mktemp -d "$install_parent/.mobius-codex-install.XXXXXX")
  trap 'rm -rf "$install_root"' EXIT
  clean_path="$(dirname "$codex_binary"):/usr/bin:/bin"
  export HOME="$install_root/home"
  export CODEX_HOME="$HOME/.codex"
  export PATH="$clean_path"
  mkdir -p "$CODEX_HOME"

  marketplace_result=$("$codex_binary" plugin marketplace add "$bundle_root" --json) \
    || fail "Codex rejected the assembled marketplace"
  jq -e '.marketplaceName == "mobius"' <<<"$marketplace_result" >/dev/null \
    || fail "Codex added an unexpected marketplace"

  available=$("$codex_binary" plugin list --marketplace mobius --available --json) \
    || fail "Codex could not list the assembled marketplace"
  jq -e '
    (.available | length) == 1 and
    .available[0].pluginId == "mobius@mobius" and
    .available[0].installPolicy == "AVAILABLE"
  ' <<<"$available" >/dev/null || fail "Codex did not admit the Mobius catalog entry"

  install_result=$("$codex_binary" plugin add mobius@mobius --json) \
    || fail "Codex rejected the assembled plugin"
  installed=$(jq -r '.installedPath' <<<"$install_result")
  case $installed in
    "$CODEX_HOME"/plugins/cache/mobius/mobius/*) ;;
    *) fail "Codex installed outside the isolated cache: $installed" ;;
  esac
  validate_config "$installed"
  smoke_installed_plugin "$installed" "$install_root"

  effective_mcp=$("$codex_binary" mcp list --json) \
    || fail "Codex could not resolve the installed plugin MCP config"
  effective_entry=$(jq -ce '
    [.[] | select(.name == "mobius" and .enabled == true)] as $entries |
    if ($entries | length) == 1 then $entries[0] else error("expected one Mobius MCP") end
  ' <<<"$effective_mcp") || fail "Codex did not expose one enabled Mobius MCP server"
  resolved_command=$(jq -r '.transport.command' <<<"$effective_entry")
  resolved_cwd=$(jq -r '.transport.cwd' <<<"$effective_entry")
  mapfile -t resolved_args < <(jq -r '.transport.args[]' <<<"$effective_entry")
  [ "$resolved_command" = "./bin/mobius" ] \
    || fail "Codex changed the installed Mobius MCP command: $resolved_command"
  [ "${#resolved_args[@]}" = "1" ] && [ "${resolved_args[0]}" = "mcp" ] \
    || fail "Codex changed the installed Mobius MCP arguments"
  [ "$(CDPATH= cd -- "$resolved_cwd" && pwd)" = "$(CDPATH= cd -- "$installed" && pwd)" ] \
    || fail "Codex resolved the Mobius MCP cwd outside the installed plugin"

  workspace="$install_root/workspace with space"
  mkdir -p "$workspace"
  workspace_uri="file://${workspace// /%20}"

  mcp_wire_call() {
    tool_name=$1
    arguments=$2
    initialize_request='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"release-host-probe","version":"1"}}}'
    initialized_notification='{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}'
    tool_request=$(jq -nc \
      --arg name "$tool_name" \
      --arg uri "$workspace_uri" \
      --argjson arguments "$arguments" \
      '{jsonrpc:"2.0",id:2,method:"tools/call",params:{name:$name,arguments:$arguments,_meta:{"codex/sandbox-state-meta":{sandboxCwd:$uri}}}}')
    wire_output=$(
      printf '%s\n%s\n%s\n' \
        "$initialize_request" "$initialized_notification" "$tool_request" \
        | (
            cd "$resolved_cwd"
            env -i HOME="$HOME" PATH=/nonexistent \
              "$resolved_command" "${resolved_args[@]}"
          )
    ) || fail "installed MCP failed during $tool_name"
    jq -s -e '([.[] | select(.id == 1)] | length) == 1 and
      .[0].result.capabilities.experimental["codex/sandbox-state-meta"] == {}' \
      <<<"$wire_output" >/dev/null \
      || fail "installed MCP did not negotiate Codex sandbox-state metadata"
    response=$(jq -ce 'select(.id == 2)' <<<"$wire_output") \
      || fail "installed MCP returned no $tool_name response"
    jq -e '.result.isError == false' <<<"$response" >/dev/null \
      || fail "installed MCP rejected $tool_name: $response"
    printf '%s\n' "$response"
  }

  mcp_apply_transition() {
    local objective=$1
    local expected_project_seq=$2
    local expected_objective_seq=$3
    local request_id=$4
    local expected_transition=$5
    local command=$6
    local arguments response
    arguments=$(jq -nc \
      --arg root "$workspace" \
      --arg project_id "$project_id" \
      --arg request_id "$request_id" \
      --argjson project_seq "$expected_project_seq" \
      --argjson objective_seq "$expected_objective_seq" \
      --argjson command "$command" '
      {
        project_root: $root,
        project_id: $project_id,
        expected_heads: {
          expected_project_seq: $project_seq,
          expected_objective_seq: $objective_seq
        },
        request_id: $request_id,
        command: $command
      }')
    response=$(mcp_wire_call mobius_apply_transition "$arguments")
    jq -e \
      --arg objective "$objective" \
      --arg transition "$expected_transition" \
      --argjson project_seq "$((expected_project_seq + 1))" \
      --argjson objective_seq "$((expected_objective_seq + 1))" '
      .result.structuredContent.objective_id == $objective and
      .result.structuredContent.transition == $transition and
      .result.structuredContent.committed_project_seq == $project_seq and
      .result.structuredContent.committed_objective_seq == $objective_seq
    ' <<<"$response" >/dev/null \
      || fail "installed MCP returned an invalid $expected_transition commit: $response"
  }

  mcp_read_objective() {
    local objective=$1
    local kind=$2
    local arguments
    arguments=$(jq -nc \
      --arg root "$workspace" \
      --arg project_id "$project_id" \
      --arg objective "$objective" \
      --arg kind "$kind" '
      {
        binding: {project_root: $root, project_id: $project_id},
        query: {kind: $kind, objective_id: $objective}
      }')
    mcp_wire_call mobius_read "$arguments"
  }

  run_installed_loop() {
    local lane=$1
    local objective=$2
    local base_project_seq=$3
    local delegated_result=$4
    local criterion="criterion-$objective"
    local stage="stage-$objective"
    local route="route-$objective"
    local attempt="attempt-$objective"
    local evidence="evidence-$objective"
    local decision="decision-$objective"
    local spec contract map command read_response context packet packet_id
    local observation provenance audit_arguments audit_response status_response

    spec=$(jq -nc \
      --arg objective "$objective" \
      --arg criterion "$criterion" '
      {
        objective: $objective,
        revision: 1,
        intended_outcome: "reach a release-gated evidence-backed Achieved state",
        criteria: {
          ($criterion): {
            id: $criterion,
            statement: "the installed full loop is observable",
            verification_rule: "inspect the frozen observation through public MCP reads",
            scope: "local"
          }
        },
        boundaries: ["release workspace only"],
        excluded_claims: ["a candidate result is already Core evidence"]
      }')
    command=$(jq -nc \
      --arg project_id "$project_id" \
      --arg objective "$objective" \
      --argjson project_seq "$base_project_seq" \
      --argjson spec "$spec" '
      {
        activate_objective: {
          objective_spec: $spec,
          confirmation: {
            project: $project_id,
            action: "activate",
            objective_spec: {objective: $objective, revision: 1},
            confirmed_payload: $spec,
            heads: {expected_project_seq: $project_seq, expected_objective_seq: 0},
            confirmed: true
          }
        }
      }')
    mcp_apply_transition \
      "$objective" "$base_project_seq" 0 "$lane-activate" activate_objective "$command"

    contract=$(jq -nc --arg criterion "$criterion" '
      {
        outcome: "an installed loop reaches a verified state",
        criteria: [$criterion],
        objective_boundaries: ["release workspace only"],
        output: "release gate observation"
      }')
    map=$(jq -nc \
      --arg objective "$objective" \
      --arg criterion "$criterion" \
      --arg stage "$stage" \
      --argjson criterion_value "$(jq -c --arg key "$criterion" '.criteria[$key]' <<<"$spec")" \
      --argjson contract "$contract" '
      {
        objective_spec: {objective: $objective, revision: 1},
        revision: 1,
        stages: {
          ($stage): {
            id: $stage,
            name: "Installed release stage",
            outcome: $contract.outcome,
            output: $contract.output,
            kind: "ordinary"
          }
        },
        criteria: {($criterion): $criterion_value},
        dependencies: [],
        priorities: {($stage): 1},
        owners: {($criterion): $stage},
        contracts: {($stage): $contract}
      }')
    command=$(jq -nc \
      --arg objective "$objective" \
      --argjson map "$map" '
      {
        install_map: {
          map: $map,
          initial_routes: {},
          cover: {
            map: {objective: $objective, revision: 1},
            objective_spec: {objective: $objective, revision: 1},
            verdict: "covered",
            rationale: "the one Stage covers the confirmed release Objective"
          },
          carry: {}
        }
      }')
    mcp_apply_transition \
      "$objective" "$((base_project_seq + 1))" 1 "$lane-install-map" install_map "$command"

    read_response=$(mcp_read_objective "$objective" current_context)
    context=$(jq -ce \
      --arg stage "$stage" \
      --argjson project_seq "$((base_project_seq + 2))" '
      select(.result.structuredContent.heads == {
        expected_project_seq: $project_seq,
        expected_objective_seq: 2
      }) |
      .result.structuredContent.result |
      select(.kind == "current_context" and .value.stage == $stage) |
      .value.context
    ' <<<"$read_response") || fail "installed MCP returned no current AcceptanceContext"

    command=$(jq -nc \
      --arg route "$route" \
      --arg stage "$stage" \
      --argjson structural "$(jq -c '.structural' <<<"$context")" '
      {
        add_route: {
          route: {
            id: $route,
            stage: $stage,
            structural_context: $structural,
            hypothesis: "the bounded installed attempt satisfies the Stage",
            assumptions: ["the isolated release workspace remains available"],
            rationale: "release-host full-loop gate"
          }
        }
      }')
    mcp_apply_transition \
      "$objective" "$((base_project_seq + 2))" 2 "$lane-add-route" add_route "$command"

    command=$(jq -nc --arg route "$route" '{select_route:{route:$route}}')
    mcp_apply_transition \
      "$objective" "$((base_project_seq + 3))" 3 "$lane-select-route" select_route "$command"

    command=$(jq -nc \
      --arg attempt "$attempt" \
      --arg route "$route" \
      --argjson context "$context" '
      {
        start_attempt: {
          attempt: {
            id: $attempt,
            route: $route,
            ordinal: 1,
            bound: {termination_condition: "one inspected frozen observation exists"},
            context: $context
          }
        }
      }')
    mcp_apply_transition \
      "$objective" "$((base_project_seq + 4))" 4 "$lane-start-attempt" start_attempt "$command"

    case $lane in
      direct)
        observation='direct main-agent observation from the installed runtime'
        provenance='main agent inspected the installed runtime directly'
        ;;
      delegated)
        # Composition semantics are owned by the Rust and native-host gates. This release gate
        # translates one already-validated observation through the installed MCP runtime.
        observation=$(jq -er '
          [.role_output.check_results[] | select(.check_id == "VK1")][0].actual
          | ltrimstr("observed: ")
          | select(length > 0)
        ' <<<"$delegated_result") \
          || fail "prevalidated delegated candidate omitted its observation"
        provenance='main agent inspected one prevalidated delegated candidate observation'
        ;;
      *) fail "unknown release E2E lane: $lane" ;;
    esac

    command=$(jq -nc \
      --arg evidence "$evidence" \
      --arg attempt "$attempt" \
      --arg criterion "$criterion" \
      --arg observation "$observation" \
      --arg provenance "$provenance" \
      --argjson context "$context" '
      {
        record_evidence: {
          evidence: {
            id: $evidence,
            subject: {attempt: $attempt},
            context: $context,
            purpose: "stage_review",
            claims: {($criterion): "supports"},
            observation: {inline: {string: $observation}},
            provenance: {string: $provenance}
          }
        }
      }')
    mcp_apply_transition \
      "$objective" "$((base_project_seq + 5))" 5 "$lane-record-evidence" record_evidence "$command"

    command=$(jq -nc \
      --arg attempt "$attempt" \
      '{seal_attempt:{attempt:$attempt,seal_reason:"submitted"}}')
    mcp_apply_transition \
      "$objective" "$((base_project_seq + 6))" 6 "$lane-seal-attempt" seal_attempt "$command"

    read_response=$(mcp_read_objective "$objective" review_material)
    packet=$(jq -ce \
      --arg attempt "$attempt" \
      --arg evidence "$evidence" \
      --argjson project_seq "$((base_project_seq + 7))" '
      select(.result.structuredContent.heads == {
        expected_project_seq: $project_seq,
        expected_objective_seq: 7
      }) |
      .result.structuredContent.result |
      select(.kind == "review_material") |
      .value.packet |
      select(.attempt == $attempt and .evidence_set == [$evidence] and .termination == "submitted")
    ' <<<"$read_response") || fail "installed MCP returned invalid Core-materialized review material"
    packet_id=$(jq -er '.id' <<<"$packet") \
      || fail "installed MCP review material omitted the Packet identity"

    command=$(jq -nc \
      --arg decision "$decision" \
      --arg packet "$packet_id" \
      --arg criterion "$criterion" '
      {
        decision: {
          decision: {
            id: $decision,
            packet: $packet,
            judgments: {($criterion): "satisfied"},
            findings: [],
            action: "accept"
          }
        }
      }')
    mcp_apply_transition \
      "$objective" "$((base_project_seq + 7))" 7 "$lane-accept" decision "$command"

    status_response=$(mcp_read_objective "$objective" status)
    jq -e \
      --arg objective "$objective" \
      --arg stage "$stage" \
      --arg decision "$decision" \
      --argjson project_seq "$((base_project_seq + 8))" '
      .result.structuredContent.heads == {
        expected_project_seq: $project_seq,
        expected_objective_seq: 8
      } and
      .result.structuredContent.result.kind == "status" and
      .result.structuredContent.result.value.active_objective == null and
      .result.structuredContent.result.value.objective_state.achieved.objective == $objective and
      .result.structuredContent.result.value.objective_state.achieved.manifest[$stage] == $decision
    ' <<<"$status_response" >/dev/null \
      || fail "installed $lane loop did not reach a fresh Achieved state"

    audit_arguments=$(jq -nc \
      --arg root "$workspace" \
      --arg project_id "$project_id" \
      '{binding:{project_root:$root,project_id:$project_id}}')
    audit_response=$(mcp_wire_call mobius_audit "$audit_arguments")
    jq -e --argjson project_seq "$((base_project_seq + 8))" '
      .result.structuredContent.status == "healthy" and
      .result.structuredContent.project_seq == $project_seq and
      .result.structuredContent.issues == []
    ' <<<"$audit_response" >/dev/null \
      || fail "installed $lane loop did not finish with a healthy audit"
  }

  init_arguments=$(jq -nc \
    --arg root "$workspace" \
    '{project_root:$root,request_id:"release-host-project-init"}')
  init_response=$(mcp_wire_call mobius_project_init "$init_arguments")
  project_id=$(jq -er '.result.structuredContent.project_id' <<<"$init_response") \
    || fail "installed MCP project_init returned no project identity"

  read_arguments=$(jq -nc \
    --arg root "$workspace" \
    --arg project_id "$project_id" \
    '{binding:{project_root:$root,project_id:$project_id},query:{kind:"status",objective_id:null}}')
  read_response=$(mcp_wire_call mobius_read "$read_arguments")
  jq -e '
    .result.structuredContent.heads == {
      "expected_objective_seq": 0,
      "expected_project_seq": 0
    } and
    .result.structuredContent.result.value.objective_ids == []
  ' <<<"$read_response" >/dev/null || fail "installed MCP read returned unexpected state"

  run_installed_loop direct release-direct 0 ''

  delegated_result=$(jq -nc '
    {
      role_output: {
        check_results: [
          {
            check_id: "VK1",
            actual: "observed: installed delegated candidate passed independent inspection"
          }
        ]
      }
    }')
  run_installed_loop delegated release-delegated 8 "$delegated_result"

  [ -f "$workspace/.mobius/mobius.sqlite3" ] \
    || fail "installed MCP did not create project-local state"
  [ ! -e "$installed/.mobius" ] \
    || fail "installed MCP wrote state into the plugin cache"
  managed_root_count=$(find "$install_root" -type d -name .mobius | wc -l | tr -d ' ')
  [ "$managed_root_count" = "1" ] \
    || fail "installed MCP created state outside the one admitted workspace"
}

validate_archive() {
  archive=$1
  checksum="$archive.sha256"
  require_file "$archive"
  require_file "$checksum"

  archive_dir=$(CDPATH= cd -- "$(dirname -- "$archive")" && pwd)
  archive_name=$(basename -- "$archive")
  expected_line=$(cd "$archive_dir" && sha256sum "$archive_name")
  actual_line=$(cat "$checksum")
  [ "$actual_line" = "$expected_line" ] \
    || fail "archive checksum must contain the exact digest and basename, without a build-host path"

  (
    cd "$archive_dir"
    sha256sum --check --strict "$archive_name.sha256" >/dev/null
  ) || fail "archive checksum verification failed"
}

if [ "$#" -ne 2 ]; then
  fail "usage: release_bundle_contract.sh <source|bundle|bundle-shape|archive|codex-install> <path>"
fi

case $1 in
  source) validate_source "$2" ;;
  bundle) validate_bundle "$2" ;;
  bundle-shape) validate_bundle_shape "$2" ;;
  archive) validate_archive "$2" ;;
  codex-install) validate_codex_install "$2" ;;
  *) fail "unknown validation mode: $1" ;;
esac
