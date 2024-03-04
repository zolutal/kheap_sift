# kheap_sift

A utility for finding Linux kernel heap objects of desired sizes.

This tool combines DWARF type information parsed from a vmlinux file using [dwat](https://github.com/zolutal/dwat), and source code pattern matching using [weggli](https://github.com/weggli-rs/weggli).

# Usage

```
Usage: kheap_sift [OPTIONS] <VMLINUX_PATH> <SOURCE_PATH> <LOWER_BOUND> <UPPER_BOUND>

Arguments:
  <VMLINUX_PATH>  The path to the vmlinux file.
  <SOURCE_PATH>   The path to the Linux source code directory.
  <LOWER_BOUND>   The lower bound for struct sizes (inclusive).
  <UPPER_BOUND>   The upper bound for struct sizes (exclusive).

Options:
      --quiet  Silence dwat/weggli output, only print struct names.
  -h, --help   Print help
```

## Example Output/Usage

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

/home/jmill/linux/kernel/bpf/arraymap.c:1109
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

```
┌──(jmill@ubun)-[~/repos/kheap_sift]
└─$ kheap_sift ~/linux/vmlinux ~/linux/ 0 64 --flags "GFP_KERNEL_ACCOUNT" --exclude '*/drivers/**/*'
======== Found allocation sites for: struct fdtable ========

struct fdtable {
    unsigned int max_fds;                       	/*    4 |    0 */
    struct file **fd;                           	/*    8 |    8 */
    long unsigned int *close_on_exec;           	/*    8 |   16 */
    long unsigned int *open_fds;                	/*    8 |   24 */
    long unsigned int *full_fds_bits;           	/*    8 |   32 */
    struct callback_head rcu;                   	/*   16 |   40 */

    /* total size: 56 */
} __attribute((__aligned__(8)));

/home/jmill/linux/fs/file.c:105
static struct fdtable * alloc_fdtable(unsigned int nr)
{
	struct fdtable *fdt;
...
		nr = ((sysctl_nr_open - 1) | (BITS_PER_LONG - 1)) + 1;

	fdt = kmalloc(sizeof(struct fdtable), GFP_KERNEL_ACCOUNT);
	if (!fdt)
		goto out;
...
out:
	return NULL;
```

# Contributing

Feel free to open issues/PRs for improvements!

# Attribution

The code in the src/wegg.rs file is largely copied with minor or no modifications from here:
https://github.com/weggli-rs/weggli/blob/main/src/main.rs

Therefore, for attribution reasons the license for weggli is included in this project as "LICENSE.weggli" because I think/hope thats how licensing works.

The rest of the code is under the BSD-2-Clause license found in "LICENSE".
