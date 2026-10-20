import unittest
import terrain_ramp_contract as gate


def listing(names):
    return ''.join(n + ': test\n' for n in names)


def output(name=gate.APP_GPU, filtered=7):
    return (f'running 1 test\ntest {name} ... ok\n'
            f'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; {filtered} filtered out; finished in 1s\n')


class Contract(unittest.TestCase):
    def test_exact_census(self):
        full = [gate.APP_CPU, gate.APP_GPU, 'legacy::must_remain']
        self.assertEqual(gate.inventories(listing(gate.CPU), '', listing(full), listing([gate.APP_GPU])), 3)
        for replacement in [[], [gate.APP_CPU], full + [gate.APP_GPU], full + [gate.PREFIX + 'unexpected']]:
            with self.assertRaises(RuntimeError):
                gate.inventories(listing(gate.CPU), '', listing(replacement), listing([gate.APP_GPU]))
        with self.assertRaises(RuntimeError):
            gate.inventories(listing(gate.CPU), listing([gate.CPU[0]]), listing(full), listing([gate.APP_GPU]))

    def test_wrong_ignored_inventory(self):
        for ignored in [[], [gate.APP_CPU], [gate.APP_GPU, gate.APP_CPU]]:
            with self.assertRaises(RuntimeError):
                gate.inventories(listing(gate.CPU), '', listing([gate.APP_CPU, gate.APP_GPU]), listing(ignored))

    def test_exact_result_and_filtered_count(self):
        gate.result(output(), [gate.APP_GPU], 8)
        for bad in [output(filtered=0), output(name=gate.APP_CPU), output().replace('1 passed', '0 passed'),
                    output().replace('0 ignored', '1 ignored'), output() + 'SKIP: no device\n', output() * 2]:
            with self.assertRaises(RuntimeError):
                gate.result(bad, [gate.APP_GPU], 8)

    def test_interleaved_stderr_requires_success_corroboration(self):
        text = output().replace('... ok\n', '... driver diagnostic\nok\nsuccesses:\n    ' + gate.APP_GPU + '\n\n')
        gate.result(text, [gate.APP_GPU], 8)
        with self.assertRaises(RuntimeError):
            gate.result(text.replace('    ' + gate.APP_GPU, '    wrong::test'), [gate.APP_GPU], 8)

    def test_software_identity_and_source(self):
        good = dict(backend='Vulkan', device_type='Cpu', adapter='llvmpipe (LLVM)', source_sha='a' * 40)
        gate.hosted_adapter(good, 'a' * 40)
        for change in [dict(backend='Gl'), dict(device_type='DiscreteGpu'), dict(adapter='SwiftShader'), dict(source_sha='b' * 40)]:
            with self.assertRaises(RuntimeError):
                gate.hosted_adapter(good | change, 'a' * 40)


if __name__ == '__main__':
    unittest.main()
