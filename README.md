# kheap_sift

A utility for finding Linux kernel heap objects of desired sizes.

This tool combines DWARF type information parsed from a vmlinux file using [dwat](https://github.com/zolutal/dwat), and source code pattern matching using [tree-sitter](https://tree-sitter.github.io/tree-sitter/).

# Usage

```
Usage: kheap_sift [OPTIONS] <VMLINUX_PATH> <SOURCE_PATH> <LOWER_BOUND> <UPPER_BOUND>

Arguments:
  <VMLINUX_PATH>  The path to the vmlinux file.
  <SOURCE_PATH>   The path to the Linux source code directory.
  <LOWER_BOUND>   The lower bound for struct sizes (exclusive).
  <UPPER_BOUND>   The upper bound for struct sizes (inclusive).

Options:
      --quiet              Silence most output, only print struct names when allocation sites are found.
      --flags <FLAGS>      Regex filter on the allocation flags argument.
      --exclude <EXCLUDE>  Glob to exclude files based on, can be specified multiple times.
      --threads <THREADS>  Number of threads to scale up to.
  -h, --help               Print help
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
└─$ kheap_sift ~/linux-6.6.7/vmlinux ~/linux-6.6.7 128 256 --exclude '*/drivers/**/*' --flags "GFP_KERNEL$" --threads 16
======== Found allocation site for: struct deflate_ctx ========

struct deflate_ctx {
    struct z_stream_s comp_stream;              	/*   96 |    0 */
    struct z_stream_s decomp_stream;            	/*   96 |   96 */

    /* total size: 192 */
};

/home/jmill/linux-6.6.7/crypto/deflate.c:115
static void *deflate_alloc_ctx(struct crypto_scomp *tfm)
...
	struct deflate_ctx *ctx;
...
	ctx = kzalloc(sizeof(*ctx), GFP_KERNEL);
...
	return ctx;
}
```

# Contributing

Feel free to open issues/PRs for improvements!

