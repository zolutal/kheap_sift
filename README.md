# kheap_sift

A utility for finding Linux kernel heap objects of desired sizes.

This tool combines DWARF type information parsed from a vmlinux file using [dwat](https://github.com/zolutal/dwat), and source code pattern matching using [weggli](https://github.com/weggli-rs/weggli).

# Usage

`Usage: kheap_sift <VMLINUX_PATH> <SOURCE_PATH> <LOWER_BOUND> <UPPER_BOUND>`

## Example Ouput

```
┌──(jmill@ubun)-[~/repos/kheap_sift]
└─$ kheap_sift ~/linux/vmlinux ~/linux 96 128
======== Found allocation sites for: struct bpf_array_aux ========

struct bpf_array_aux {
    struct list_head poke_progs;                	/*   16 |    0 */
    struct bpf_map *map;                        	/*    8 |   16 */
    struct mutex poke_mutex;                    	/*   56 |   24 */
    struct work_struct work;                    	/*   32 |   80 */

    /* total size: 112 */
};

/home/jmill/kernel-junk/linux-6.1-rc3/kernel/bpf/arraymap.c:1109
static struct bpf_map *prog_array_map_alloc(union bpf_attr *attr)
{
	struct bpf_array_aux *aux;
	struct bpf_map *map;

	aux = kzalloc(sizeof(*aux), GFP_KERNEL_ACCOUNT);
	if (!aux)
		return ERR_PTR(-ENOMEM);
...

	return map;
```

# Contributing

Feel free to open issues/PRs for improvements!

# Attribution

The code in the src/wegg.rs file is largely copied with minor or no modifications from here:
https://github.com/weggli-rs/weggli/blob/main/src/main.rs

Therefore, for attribution reasons the license for weggli is included in this project as "LICENSE.weggli" because I think/hope thats how licensing works.

The rest of the code is under the BSD-2-Clause license found in "LICENSE".