import copy
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import promote_validated_release as promoter
import verify_linux_native_payloads as linux


def successful_inventory():
    run = {'id': promoter.RUN, 'status': 'completed', 'conclusion': 'success', 'head_sha': promoter.HEAD,
           'head_branch': 'main', 'path': '.github/workflows/release.yml', 'event': 'workflow_dispatch',
           'repository': {'full_name': promoter.REPOSITORY}}
    jobs = [{'name': name, 'status': 'completed', 'conclusion': 'success',
             'steps': [{'name': step, 'status': 'completed', 'conclusion': 'success'} for step in steps]}
            for name, steps in promoter.REQUIRED_JOBS.items()]
    main = {'object': {'sha': promoter.HEAD, 'type': 'commit'}}
    return run, jobs, main


class RefusalTests(unittest.TestCase):
    def test_exact_successful_inventory(self):
        promoter.validate_run(*successful_inventory())

    def test_unsuccessful_incomplete_wrong_run_or_sha(self):
        for field, value in [('conclusion', 'failure'), ('status', 'in_progress'), ('id', promoter.RUN + 1),
                             ('head_sha', 'f' * 40), ('head_branch', 'other'), ('event', 'push'),
                             ('path', '.github/workflows/ci.yml')]:
            with self.subTest(field=field):
                run, jobs, main = successful_inventory()
                run[field] = value
                with self.assertRaises(RuntimeError): promoter.validate_run(run, jobs, main)

    def test_main_moved_or_wrong_repository(self):
        run, jobs, main = successful_inventory()
        main['object']['sha'] = 'f' * 40
        with self.assertRaises(RuntimeError): promoter.validate_run(run, jobs, main)
        run, jobs, main = successful_inventory()
        run['repository']['full_name'] = 'other/repo'
        with self.assertRaises(RuntimeError): promoter.validate_run(run, jobs, main)

    def test_every_required_job_is_required(self):
        run, jobs, main = successful_inventory()
        self.assertEqual(len(jobs), 11)
        for index in range(len(jobs)):
            with self.subTest(job=jobs[index]['name']):
                with self.assertRaises(RuntimeError): promoter.validate_run(run, jobs[:index] + jobs[index+1:], main)
                changed = copy.deepcopy(jobs)
                changed[index]['conclusion'] = 'skipped'
                with self.assertRaises(RuntimeError): promoter.validate_run(run, changed, main)

    def test_all_mandatory_steps_cannot_skip_or_fail(self):
        run, jobs, main = successful_inventory()
        for index, job in enumerate(jobs):
            for step_index, step in enumerate(job['steps']):
                for result in ['skipped', 'failure']:
                    with self.subTest(job=job['name'], step=step['name'], result=result):
                        changed = copy.deepcopy(jobs)
                        changed[index]['steps'][step_index]['conclusion'] = result
                        with self.assertRaises(RuntimeError): promoter.validate_run(run, changed, main)

    def test_duplicate_job_step_and_unexpected_job(self):
        run, jobs, main = successful_inventory()
        for changed in [jobs + [jobs[0]], jobs + [{'name': 'unreviewed', 'conclusion': 'success'}]]:
            with self.assertRaises(RuntimeError): promoter.validate_run(run, changed, main)
        jobs[0]['steps'].append(jobs[0]['steps'][0])
        with self.assertRaises(RuntimeError): promoter.validate_run(run, jobs, main)

    def test_only_known_dry_run_remote_writers_may_skip(self):
        run, jobs, main = successful_inventory()
        for name in ['publish', 'repositories', 'repositories / apt']:
            promoter.validate_run(run, jobs + [{'name': name, 'conclusion': 'skipped'}], main)
            with self.assertRaises(RuntimeError): promoter.validate_run(run, jobs + [{'name': name, 'conclusion': 'success'}], main)

    def test_repeated_nonmandatory_download_action_names_are_allowed(self):
        run, jobs, main = successful_inventory()
        step = {'name': 'Run actions/download-artifact@pinned', 'status': 'completed', 'conclusion': 'success'}
        jobs[-1]['steps'].extend([step.copy(), step.copy()])
        promoter.validate_run(run, jobs, main)

    def test_dispatch_refuses_before_any_network_or_mutation(self):
        for channel, dry_run in [('nightly', 'false'), ('production', 'true'), ('production', ''), ('', 'false')]:
            with patch.dict(promoter.os.environ, {'PROMOTE_CHANNEL': channel, 'PROMOTE_DRY_RUN': dry_run}, clear=True), patch.object(promoter, 'command') as command, patch.object(promoter, 'api') as api:
                with self.assertRaises(RuntimeError): promoter.validate(Path('/unused'), Path('/unused'))
                command.assert_not_called()
                api.assert_not_called()

    def test_graphql_error_response_cannot_prove_absence(self):
        with self.assertRaises(RuntimeError):
            promoter.validate_release_absent({'data': {'repository': {'ref': None, 'release': None}}, 'errors': [{'message': 'permission denied'}]})

    def test_existing_release_or_tag_refuses(self):
        promoter.validate_release_absent({'data': {'repository': {'ref': None, 'release': None}}})
        for repository in [{'ref': {'target': {'oid': 'f'*40}}, 'release': None},
                           {'ref': None, 'release': {'id': 1}}, {}, {'ref': None}, {'release': None}, None]:
            with self.assertRaises(RuntimeError): promoter.validate_release_absent({'data': {'repository': repository}})


class PreviewTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.assets = self.root / 'assets'; self.assets.mkdir()
        self.preview = self.root / 'preview'; self.preview.mkdir()
        self.receipt = {'assets': {}}
        lines = []
        for index in range(10):
            name = f'asset-{index}.zip'
            data = bytes([index])
            (self.assets / name).write_bytes(data)
            checksum = hashlib.sha256(data).hexdigest()
            self.receipt['assets'][name] = {'sha256': checksum}
            lines.append(f'{checksum}  ./{name}')
        for name in ['release.json', f'oxplay-{promoter.TAG}-source.tar.gz']:
            lines.append(f'{"a"*64}  ./{name}')
        self.sums = self.preview / 'SHA256SUMS.txt'
        self.sums.write_text('\n'.join(lines) + '\n')
        self.plan = self.preview / 'release-plan.json'
        self.plan.write_text(json.dumps({'schema':1,'channel':'production','dry_run':True,'git_head':promoter.HEAD,'tag':promoter.TAG,'version':'0.1.0'}))

    def test_standard_sha256sum_dot_slash_and_all_ten_hashes(self):
        promoter.compare_preview(self.assets, self.preview, self.receipt)

    def test_altered_payload_or_preview_checksum_refuses(self):
        (self.assets / 'asset-0.zip').write_bytes(b'changed')
        with self.assertRaises(RuntimeError): promoter.compare_preview(self.assets, self.preview, self.receipt)
        (self.assets / 'asset-0.zip').write_bytes(b'\0')
        self.sums.write_text(self.sums.read_text().replace(self.receipt['assets']['asset-0.zip']['sha256'], 'b'*64))
        with self.assertRaises(RuntimeError): promoter.compare_preview(self.assets, self.preview, self.receipt)

    def test_duplicate_normalized_path_and_unsafe_name_refuse(self):
        original = self.sums.read_text()
        for line in ['a'*64+'  asset-0.zip', 'a'*64+'  ../outside', 'a'*64+'  /absolute']:
            self.sums.write_text(original + line + '\n')
            with self.assertRaises(RuntimeError): promoter.checksum_manifest(self.sums)

    def test_wrong_preview_head_or_non_dry_run_refuses(self):
        for field, value in [('git_head', 'f'*40), ('dry_run',False), ('channel','nightly'), ('version','0.2.0')]:
            plan=json.loads(self.plan.read_text()); original=plan.copy(); plan[field]=value; self.plan.write_text(json.dumps(plan))
            with self.assertRaises(RuntimeError): promoter.compare_preview(self.assets,self.preview,self.receipt)
            self.plan.write_text(json.dumps(original))


class LinuxBoundsTests(unittest.TestCase):
    def test_no_unsafe_member_paths(self):
        for name in ['/absolute','../escape','a/../../escape','a\\b']:
            with self.assertRaises(RuntimeError): linux.safe_name(name)
        self.assertEqual(linux.safe_name('./usr/lib/oxplay/libmpv.so.2'), 'usr/lib/oxplay/libmpv.so.2')

    def test_stream_digest_is_bounded(self):
        with self.assertRaises(RuntimeError): linux.digest(io.BytesIO(b'abcdef'), bound=5)
        self.assertEqual(linux.digest(io.BytesIO(b'abc')), hashlib.sha256(b'abc').hexdigest())

    def test_invalid_appimage_is_rejected_without_execution(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'fake.AppImage'; path.write_bytes(b'not-elf')
            with self.assertRaises(RuntimeError): linux.squashfs_offset(path)

    def test_actual_safe_package_reader_preserves_bytes_without_extraction(self):
        import tarfile
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'tiny.tar.gz'
            name='usr/share/doc/oxplay/oxplay-native/native-media-provenance.json'
            data=b'{"native_render_abi":1}'
            with tarfile.open(path, 'w:gz') as tar:
                info=tarfile.TarInfo(name); info.size=len(data); tar.addfile(info,io.BytesIO(data))
            archive=linux.Archive(path)
            self.assertEqual(archive.read(name),data)
            self.assertEqual(archive.checksums({name:'unused'}), {name:hashlib.sha256(data).hexdigest()})
            self.assertEqual(list(Path(directory).iterdir()),[path])

    def test_missing_native_receipt_and_duplicate_members_refuse(self):
        import tarfile
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'tiny.tar.gz'
            for names in [['usr/bin/legacy'], ['usr/bin/duplicate','./usr/bin/duplicate']]:
                with tarfile.open(path,'w:gz') as tar:
                    for name in names:
                        info=tarfile.TarInfo(name); info.size=1; tar.addfile(info,io.BytesIO(b'x'))
                with self.assertRaises(RuntimeError): linux.Archive(path)

    def test_payload_verification_precedes_remote_mutation_and_ref_checks_repeat(self):
        text=(Path(__file__).parent/'promote_validated_release.py').read_text()
        body=text[text.index('def promote(source, evidence):'):]
        self.assertLess(body.index('verify_release_assets.py'),body.index("'commit-version'"))
        self.assertLess(body.index('compare_preview(assets'),body.index("'commit-version'"))
        self.assertLess(body.index('verify_linux_native_payloads.py'),body.index("'commit-version'"))
        self.assertLess(body.index('bytes differ from the binary build'),body.index("'commit-version'"))
        self.assertGreater(body.index('Actual main changed before publication'),body.index("'commit-version'"))
        self.assertLess(body.index('Actual main changed before publication'),body.index("'publish'"))
        self.assertGreater(body.rindex('require_release_absent()'),body.index("'commit-version'"))

    def test_workflow_uses_exact_control_source_and_run(self):
        workflow=(Path(__file__).resolve().parents[1]/'.github/workflows/release.yml').read_text()
        self.assertIn(f'ref: {promoter.HEAD}',workflow)
        self.assertEqual(workflow.count(f"run-id: '{promoter.RUN}'"),2)
        for value in ['path: controls','path: source','fetch-depth: 0','permissions:\n  contents: write\n  actions: read','group: oxplay-release-publish','github-token: ${{ github.token }}']:
            self.assertIn(value,workflow)
        self.assertNotIn('cargo build',workflow)
        for action, pin in [('checkout','3d3c42e5aac5ba805825da76410c181273ba90b1'),('setup-python','a309ff8b426b58ec0e2a45f0f869d46889d02405'),('download-artifact','3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c'),('upload-artifact','043fb46d1a93c77aae656e7c1c64a875d1fc6a0a')]:
            self.assertIn(f'actions/{action}@{pin}',workflow)


if __name__ == '__main__':
    unittest.main()
