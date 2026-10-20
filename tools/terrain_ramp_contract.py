"""Staged issue260 census contract only; not an executable acceptance gate."""
import re
from tilemap_authoring_gate import SUMMARY, validate_cases, validate_inventory

CPU = [
    'grouped_fixed_arrays_axes_signs_fractional_ties_reversal_and_preservation',
    'grouped_edges_invalid_shapes_hole_halo_limits_and_noop_preserve_source',
    'actual_session_single_transaction_noop_undo_redo_failure_save_and_fresh_reopen',
]
PREFIX = 'app::terrain_ramp_tests::'
APP_CPU = PREFIX + 'app_context_refusals_preserve_document_history_and_original_files'
APP_GPU = PREFIX + 'actual_viewport_ramp_navigation_save_reopen_play_and_stale_refusal'


def require(value, message):
    if not value:
        raise RuntimeError(message)


def inventories(integration_all, integration_ignored, library_all, library_ignored):
    validate_inventory(integration_all, CPU)
    validate_inventory(integration_ignored, [])
    def names(output):
        result = re.findall(r'^(\S+): test$', output.replace('\r\n', '\n'), re.M)
        require(result and len(result) == len(set(result)), 'Missing or duplicate full library census')
        return result
    full, ignored = names(library_all), names(library_ignored)
    require(set(ignored) <= set(full), 'Ignored inventory outside full census')
    validate_inventory('\n'.join(n + ': test' for n in full if n.startswith(PREFIX)), [APP_CPU, APP_GPU])
    validate_inventory('\n'.join(n + ': test' for n in ignored if n.startswith(PREFIX)), [APP_GPU])
    return len(full)


def result(output, expected, total):
    validate_cases(output, expected)
    require(type(total) is int and total >= len(expected), 'Invalid total census')
    require(int(SUMMARY.findall(output.replace('\r\n', '\n'))[0][4]) == total - len(expected),
            'Incorrect filtered count')


def hosted_adapter(value, source_sha):
    require(value.get('backend') == 'Vulkan' and value.get('device_type') == 'Cpu',
            'Vulkan Cpu adapter required')
    require(isinstance(value.get('adapter'), str) and 'llvmpipe' in value['adapter'].lower(),
            'Hosted llvmpipe required; local SwiftShader evidence is separate')
    require(re.fullmatch('[0-9a-f]{40}|[0-9a-f]{64}', source_sha) is not None and
            value.get('source_sha') == source_sha, 'Source receipt mismatch')
