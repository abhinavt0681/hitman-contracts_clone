"""Exploration helper for Glacier 1 compiled scripts (.ppo inside <scene>_MAIN.ZGF)."""
import re, struct, sys

def u32(b, o): return struct.unpack_from('<I', b, o)[0]

def scripts(zgf):
    starts = [m.start() for m in re.finditer(rb'PPO! \x00\x00\x00', zgf)]
    for s in starts:
        _, hdr, ver, mid_len, iimv_len, _, schema_end, iimv_at = struct.unpack_from('<4sIfIIIII', zgf, s)
        iimv = s + iimv_at
        assert zgf[iimv:iimv + 4] == b'IIMV'
        secs = struct.unpack_from('<12I', zgf, iimv + 0x54)
        img = iimv + 0x84
        img_len = secs[11]
        src = zgf[img:zgf.index(b'\0', img)].decode()
        yield dict(at=s, iimv=iimv, iimv_len=iimv_len, img=img, img_len=img_len, secs=secs[:11], src=src)

if __name__ == '__main__':
    zgf = open(sys.argv[1], 'rb').read()
    for sc in scripts(zgf):
        img = sc['img']
        print(sc['src'], 'img', hex(sc['img_len']), 'secs', [hex(x) for x in sc['secs']])
        for k, off in enumerate(sc['secs']):
            if off == 0 and k != 0: continue
            o = img + off
            print('   sec+%02x @%05x: %s' % (0x64 + 4 * k, off, zgf[o:o + 24].hex(' ')))
        code_len = u32(zgf, img + sc['secs'][10])
        print('   code len', hex(code_len), 'ends', hex(sc['secs'][10] + 4 + code_len), 'img_len', hex(sc['img_len']))
