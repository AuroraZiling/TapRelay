const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { createHash } = require('node:crypto');
const { publish, releaseAssets } = require('../publish-release.cjs');

function fixture(t) {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'taprelay-release-'));
    t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
    for (const suffix of ['Windows_x64_Setup.exe', 'Windows_x64.zip', 'macOS_arm64.pkg', 'macOS_x64.pkg']) {
        fs.writeFileSync(path.join(dir, 'TapRelay_1.0.0_' + suffix), suffix);
    }
    for (const channel of ['win', 'osx-arm64', 'osx-x64']) {
        const name = `TapRelay-1.0.0${channel === 'win' ? '' : '-' + channel}-full.nupkg`;
        fs.writeFileSync(path.join(dir, name), channel);
        fs.writeFileSync(path.join(dir, `releases.${channel}.json`), JSON.stringify({
            Assets: [{
                PackageId: 'TapRelay', Version: '1.0.0', Type: 'Full', FileName: name, Size: channel.length,
                SHA256: createHash('sha256').update(channel).digest('hex').toUpperCase(),
            }],
        }));
    }
    return dir;
}

test('rejects a replaced payload before publishing its original feed', t => {
    const dir = fixture(t);
    fs.writeFileSync(path.join(dir, 'TapRelay-1.0.0-osx-arm64-full.nupkg'), 'different');
    assert.throws(() => releaseAssets(dir, '1.0.0'), /does not match releases.osx-arm64.json/);
});

const context = { repo: { owner: 'owner', repo: 'TapRelay' }, sha: 'commit' };
const git = { getRef: async () => ({ data: { object: { type: 'commit', sha: context.sha } } }) };

test('does not touch a previously published version', async t => {
    const repos = { getReleaseByTag: async () => ({ data: { draft: false } }) };
    await assert.rejects(publish({ rest: { git, repos } }, context, 'v1.0.0', '1.0.0', fixture(t)), /already published/);
});

test('resumes a draft and publishes only after all ten assets are uploaded', async t => {
    const dir = fixture(t);
    const uploaded = [];
    const repos = {
        getReleaseByTag: async () => { throw Object.assign(new Error(), { status: 404 }); },
        listReleases() {},
        listReleaseAssets() {},
        deleteReleaseAsset: async ({ asset_id }) => assert.equal(asset_id, 2),
        uploadReleaseAsset: async ({ name }) => uploaded.push(name),
        updateRelease: async ({ draft }) => {
            assert.equal(draft, false);
            assert.deepEqual(uploaded.toSorted(), fs.readdirSync(dir).sort());
        },
    };
    const paginate = async method => method === repos.listReleases
        ? [{ draft: true, tag_name: 'v1.0.0', id: 1 }]
        : [{ id: 2, name: 'releases.win.json' }];
    await publish({ rest: { git, repos }, paginate }, context, 'v1.0.0', '1.0.0', dir);
    assert.equal(uploaded.length, 10);
});
