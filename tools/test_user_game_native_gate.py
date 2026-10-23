"""Synthetic source/provenance negatives, NOT native acceptance evidence."""
import json
import ast
import inspect
import types
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

import static_game_native_gate as native
import user_game_native_gate as gate


class UserGateTests(unittest.TestCase):
    def test_shared_budget_driver_wiring_and_phase_routing(self):
        # Execute the actual orchestration statements, not a duplicated fixture.
        tree = ast.parse(inspect.getsource(gate.run))
        body = next(n for n in ast.walk(tree) if isinstance(n, ast.Try)).body
        statements = [n for n in body if
            (isinstance(n, ast.Assign) and any(isinstance(t, ast.Name) and t.id == 'compile_budget' for t in n.targets)) or
            (isinstance(n, ast.Expr) and isinstance(n.value, ast.Call) and
             isinstance(n.value.func, ast.Attribute) and n.value.func.attr == 'run' and
             isinstance(n.value.func.value, ast.Name) and n.value.func.value.id in ('editor', 'standalone'))]
        self.assertEqual(len(statements), 3)
        editor, standalone = mock.Mock(), mock.Mock()
        namespace = dict(editor=editor, standalone=standalone, artifacts=Path('/evidence'), context=object(), env={})
        exec(compile(ast.Module(body=statements, type_ignores=[]), '<actual driver wiring>', 'exec'), namespace)
        editor.CompileBudget.assert_called_once_with()
        shared = editor.CompileBudget.return_value
        self.assertIs(editor.run.call_args.kwargs['compile_budget'], shared)
        self.assertIs(standalone.run.call_args.kwargs['compile_budget'], shared)
        for module, phases in ((native, {'compile'}), (gate.standalone, {'compile', 'editor-compile'})):
            tree = ast.parse(inspect.getsource(module.run))
            execute = next(n for n in ast.walk(tree) if isinstance(n, ast.FunctionDef) and n.name == 'execute')
            for budget in (None, mock.Mock()):
                runner = mock.Mock()
                namespace = dict(command=runner, compile_budget=budget, context=types.SimpleNamespace(project_root=Path('/project')))
                exec(compile(ast.Module(body=[execute], type_ignores=[]), '<actual phase routing>', 'exec'), namespace)
                for phase in ('compile', 'editor-compile', 'native', 'context-tests', 'inventory'):
                    runner.reset_mock()
                    if budget is not None:
                        budget.reset_mock()
                    namespace['execute'](['cargo'], Path('/evidence'), phase, {}, 900)
                    if budget is not None and phase in phases:
                        budget.command.assert_called_once_with(runner, ['cargo'], Path('/evidence'), phase, {}, Path('/project'))
                        runner.assert_not_called()
                    else:
                        runner.assert_called_once_with(['cargo'], Path('/evidence'), phase, {}, 900, cwd=Path('/project'))
                        if budget is not None:
                            budget.command.assert_not_called()

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.project = self.root/'user game'
        self.project.mkdir()
        for name in ('src/lib.rs','tests/a.rs','scenes/a.yaml','Cargo.toml','Cargo.lock'):
            path=self.project/name;path.parent.mkdir(exist_ok=True);path.write_text('fixture\n')
        (self.project/'.gitignore').write_text('ignored*\n')
        self.sha=gate.own_commit(self.project,os.environ)

    def tearDown(self):
        self.temp.cleanup()

    def fence(self):
        return native.clean_checkout(self.sha,self.project,complete=True)

    def test_python_loader_environment_is_still_rejected(self):
        home = self.root/'cargo-home'; home.mkdir()
        clean = {'CARGO_HOME': str(home), 'PATH': '/usr/bin', 'GITHUB_SHA': 'a'*40}
        original = dict(clean)
        result = gate.build_environment(clean, self.root/'target')
        self.assertEqual(clean, original)
        self.assertEqual(result['PATH'], clean['PATH'])
        self.assertNotIn('LD_LIBRARY_PATH', result)
        for value in ('', '/injected/lib', '/opt/hostedtoolcache/Python/3.12.15/x64/lib',
                      '/opt/hostedtoolcache/Python/3.12.15/x64/lib:/injected/lib',
                      '/injected/lib:/opt/hostedtoolcache/Python/3.12.15/x64/lib'):
            poisoned = dict(clean, LD_LIBRARY_PATH=value)
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, 'ambient compiler configuration LD_LIBRARY_PATH'):
                gate.build_environment(poisoned, self.root/'target')
            self.assertEqual(poisoned['LD_LIBRARY_PATH'], value)

    def test_workflow_uses_python_output_without_mutating_compiler_environment(self):
        text = (Path(__file__).resolve().parents[1]/'.github/workflows/determinism.yml').read_text()
        def contract(value):
            job = value.split('  generated-user-game-native:\n', 1)[1].split('  sample-room:\n', 1)[0]
            setup = job.split('      - uses: actions/setup-python@v5\n', 1)[1].split('      - ', 1)[0]
            self.assertEqual(setup, '        id: generated_python\n        timeout-minutes: 2\n        with:\n          python-version: "3.12"\n          update-environment: false\n')
            launch = job.split('      - name: Required generated project native editor and delivered games\n', 1)[1].split('      - name:', 1)[0]
            self.assertIn('        env:\n          ORR_ACCEPTANCE_PYTHON: ${{ steps.generated_python.outputs.python-path }}\n', launch)
            self.assertIn('          "$ORR_ACCEPTANCE_PYTHON" -B -m unittest discover -s tools -p', launch)
            self.assertIn('          "$ORR_ACCEPTANCE_PYTHON" -B tools/user_game_native_gate.py', launch)
            self.assertEqual(launch.count('"$ORR_ACCEPTANCE_PYTHON"'), 2)
            for forbidden in ('python3 -B', 'env -u', 'unset LD_', 'LD_LIBRARY_PATH:', 'LD_PRELOAD:'):
                self.assertNotIn(forbidden, launch)
        contract(text)
        for old, new in (
            ('update-environment: false', 'update-environment: true'),
            ('          update-environment: false\n', ''),
            ('id: generated_python', 'id: other_python'),
            ('steps.generated_python.outputs.python-path', 'steps.other_python.outputs.python-path'),
            ('"$ORR_ACCEPTANCE_PYTHON" -B -m unittest', 'python3 -B -m unittest'),
            ('"$ORR_ACCEPTANCE_PYTHON" -B tools/user_game_native_gate.py', 'python3 -B tools/user_game_native_gate.py'),
            ('          ROOT="$RUNNER_TEMP/generated-user-native-job"\n', '          unset LD_LIBRARY_PATH\n          ROOT="$RUNNER_TEMP/generated-user-native-job"\n'),
        ):
            # Restrict mutation to this job so sibling setup steps cannot satisfy it.
            start = text.index('  generated-user-game-native:\n')
            end = text.index('  sample-room:\n', start)
            job = text[start:end]
            self.assertIn(old, job)
            changed = text[:start]+job.replace(old, new)+text[end:]
            with self.subTest(old=old), self.assertRaises(AssertionError):
                contract(changed)

    def test_clean_generated_root_with_spaces(self):
        value=self.fence()
        self.assertEqual(value['commit'],self.sha)
        self.assertEqual(subprocess.check_output(['git','remote'],cwd=self.project),b'')

    def test_unknown_ignored_inputs_anywhere(self):
        for name in ('src/ignored.rs','tests/ignored.rs','scenes/ignored.yaml','ignored.rs','build.rs','.cargo/config.toml'):
            with self.subTest(name=name):
                path=self.project/name;path.parent.mkdir(exist_ok=True);path.write_text('injected')
                with self.assertRaises(ValueError):self.fence()
                path.unlink()

    def test_mode_change_even_when_git_ignores_modes(self):
        subprocess.run(['git','config','core.fileMode','false'],cwd=self.project,check=True)
        path=self.project/'src/lib.rs';path.chmod(0o755)
        with self.assertRaisesRegex(ValueError,'mode'):self.fence()

    def test_assume_unchanged_source_bytes(self):
        subprocess.run(['git','update-index','--assume-unchanged','src/lib.rs'],cwd=self.project,check=True)
        (self.project/'src/lib.rs').write_text('different')
        with self.assertRaisesRegex(ValueError,'bytes'):self.fence()

    def test_symlink_source_directory(self):
        source=self.project/'src';source.rename(self.root/'other');source.symlink_to(self.root/'other',target_is_directory=True)
        with self.assertRaises(ValueError):self.fence()

    def test_ancestor_configuration(self):
        for name in ('.cargo/config','.cargo/config.toml','rust-toolchain','rust-toolchain.toml'):
            path=self.root/name;path.parent.mkdir(exist_ok=True);path.write_text('injected')
            with self.assertRaisesRegex(ValueError,'ancestor'):self.fence()
            path.unlink()

    def test_output_root_overlap(self):
        for other in (self.project,self.project/'target',self.root):
            with self.assertRaisesRegex(ValueError,'overlapping'):gate.separate_roots(self.project,other)

    def test_symlink_output_root(self):
        link=self.root/'link';link.symlink_to(self.project,target_is_directory=True)
        with self.assertRaises(ValueError):gate.separate_roots(link,self.root/'evidence')

    def test_build_configuration_injection(self):
        for key in ('RUSTC','RUSTC_WRAPPER','RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','CARGO_BUILD_RUSTC','CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER','CARGO_PROFILE_RELEASE_OPT_LEVEL','CARGO_SOURCE_CRATES_IO_REPLACE_WITH','LD_PRELOAD','LD_LIBRARY_PATH','CARGO_TARGET_DIR'):
            with self.subTest(key=key),self.assertRaisesRegex(ValueError,'ambient'):gate.build_environment({key:'injected'},self.root/'target')

    def test_cargo_home_configuration(self):
        home=self.root/'cargo';home.mkdir();(home/'config.toml').write_text('injected')
        with self.assertRaises(ValueError):gate.build_environment({'CARGO_HOME':str(home)},self.root/'target')

    def context(self):
        return native.ExecutionContext(self.project,'user','path+file:///user#user@0.1.0',{'native_window':self.project/'tests/a.rs'},self.sha,self.root/'gate','a'*40)

    def test_exact_compiler_artifact(self):
        context=self.context()
        item={'package_id':context.package_id,'target':{'name':'native_window','kind':['test'],'src_path':str(self.project/'tests/a.rs')}}
        context.admit(item,'native_window','test')
        for field,value in [('package_id','another'),('name','different'),('kind',['bin']),('src_path','/engine/tests/a.rs')]:
            changed=json.loads(json.dumps(item));(changed if field=='package_id' else changed['target'])[field]=value
            with self.subTest(field=field),self.assertRaises(ValueError):context.admit(changed,'native_window','test')

    def test_original_engine_rejects_context_injection(self):
        for key in ('ORR_ACCEPTANCE_CONTEXT','ORR_ACCEPTANCE_CONTEXT_SHA256'):
            with self.assertRaises(ValueError):self.context().environment({'GITHUB_SHA':'a'*40,key:'injected'})

    def test_gate_user_sha_swap(self):
        with self.assertRaises(ValueError):self.context().environment({'GITHUB_SHA':self.sha})

    def test_context_missing_receipt_binding(self):
        context=self.context();path=self.root/'context.json';path.write_text('{}');context.context_path=path
        (self.root/'attempt.json').write_text('{}')
        with self.assertRaisesRegex(ValueError,'binding'):context.validate_binding(self.root)

    def test_own_lock_mutation(self):
        (self.project/'Cargo.lock').write_text('changed')
        with self.assertRaises(ValueError):self.fence()

    def test_complete_fence_requires_explicit_output_exemption(self):
        target=self.project/'target';target.mkdir();(target/'unknown').write_text('build output')
        with self.assertRaises(ValueError):self.fence()
        native.clean_checkout(self.sha,self.project,complete=True,output_roots=[target])

    def test_overlay_does_not_allow_production_source(self):
        self.assertTrue(all(p.startswith('tests/') for p in gate.OVERLAY))
        self.assertEqual(set(gate.FIXTURES),{'delivery','switchboard','editor_delivery','editor_switchboard'})

    def test_project_package_cannot_alias_engine(self):
        metadata={'packages':[{'name':gate.PACKAGE,'id':'id','manifest_path':'/engine/Cargo.toml'}]}
        with self.assertRaisesRegex(ValueError,'manifest'):
            gate.package_context(metadata,self.project,self.sha,self.root/'gate','a'*40,self.fence,None,[])

    def test_overlay_preserves_production_and_records_before_after(self):
        source = Path(__file__).resolve().parents[1]
        starter = source/gate.STARTER
        snapshot = {p.relative_to(starter).as_posix(): p.read_bytes() for p in starter.rglob('*') if p.is_file()}
        generated = self.root/'new user project'
        generated.mkdir()
        files = gate.bootstrap.generated_files(snapshot, self.root/'engine', gate.PACKAGE)
        for relative,data in files.items():
            path=generated/relative;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(data)
        before={p:p.read_bytes() for p in (generated/'src').rglob('*') if p.is_file()}
        receipt=gate.overlay(generated,source)
        self.assertTrue(all(p.read_bytes()==data for p,data in before.items()))
        self.assertEqual({r['path'] for r in receipt['gate_source_files']},set(gate.OVERLAY))
        for record in receipt['gate_source_files']:
            self.assertEqual(record['sha256'],record['gate_source_sha256'])
            self.assertIn('before_sha256',record)
        (generated/'Cargo.lock').write_text('owned lock fixture')
        sha=gate.own_commit(generated,os.environ)
        native.clean_checkout(sha,generated,complete=True)
        (generated/'src/lib.rs').write_text('runtime edit injected after overlay')
        with self.assertRaises(ValueError):native.clean_checkout(sha,generated,complete=True)

    def test_artifact_must_be_inside_target_and_not_linked(self):
        context=self.context();target=self.root/'target';target.mkdir();context.target_root=target
        artifact=target/'test executable';artifact.write_bytes(b'fixture')
        self.assertEqual(context.artifact_path(str(artifact)),artifact)
        outside=self.root/'outside';outside.write_bytes(b'outside')
        with self.assertRaises(ValueError):context.artifact_path(str(outside))
        linked=target/'linked';linked.symlink_to(artifact)
        with self.assertRaises(ValueError):context.artifact_path(str(linked))

    def test_exact_library_producer(self):
        context=self.context();context.target_sources['orr_static_starter']=self.project/'src/lib.rs'
        output=self.root/'libfixture.rlib';output.write_bytes(b'library')
        item={'reason':'compiler-artifact','package_id':context.package_id,
              'target':{'name':'orr_static_starter','kind':['lib'],'src_path':str(self.project/'src/lib.rs')},
              'filenames':[str(output)]}
        self.assertEqual(context.library_artifact(json.dumps(item))['package_id'],context.package_id)
        for field,value in [('package_id','wrong'),('src_path','/engine/src/lib.rs')]:
            changed=json.loads(json.dumps(item));(changed if field=='package_id' else changed['target'])[field]=value
            with self.assertRaises(ValueError):context.library_artifact(json.dumps(changed))
        with self.assertRaises(ValueError):context.library_artifact(json.dumps(item)+'\n'+json.dumps(item))

    def test_context_inventory_cannot_hide_helper_tests(self):
        names=sorted(native.CONTEXT_TESTS)
        text='\n'.join('test '+name+' ... ok' for name in names)+'\ntest result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out'
        native.context_test_output(text,1)
        with self.assertRaises(ValueError):native.context_test_output(text.replace(names[0],'wrong'),1)
        with self.assertRaises(ValueError):native.context_test_output(text.replace('3 passed','0 passed'),1)

    def test_missing_hidden_context_file_is_rejected_independently(self):
        from test_static_game_standalone_gate import fixture
        evidence=self.root/'synthetic';evidence.mkdir()
        provenance=fixture(evidence)
        provenance['hidden_source_roots']=['/hidden/execution-context.json']
        with self.assertRaisesRegex(ValueError,'hiding'):
            gate.standalone.validate(evidence,provenance)

    def test_generated_context_automatically_hides_context_file(self):
        path=self.root/'context.json';path.write_text('{}')
        context=native.ExecutionContext(self.project,'user','id',{},self.sha,self.root/'gate','a'*40,
                                        context_path=path,hidden_roots=[self.project])
        self.assertIn(str(path),context.hidden_roots)


if __name__=='__main__':unittest.main()
