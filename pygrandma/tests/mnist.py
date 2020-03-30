import numpy as np
from sklearn.neighbors import KDTree
import pygrandma
import pandas as pd

data = np.memmap("../data/mnist.dat", dtype=np.float32)
data = data.reshape([-1,784])

tree = pygrandma.PyGrandma()
tree.set_cutoff(0)
tree.set_scale_base(1.2)
tree.set_resolution(-30)
tree.fit(data)

print(tree.knn(data[0],5))

for i,layer in enumerate(tree.layers()):
    print(f"On layer {layer.scale_index()}")
    if i < 2:
        for node in layer.nodes():
            print(f"\tNode {node.address()} mean: {node.cover_mean().mean()}")

print("============= TRACE =============")
trace = tree.dry_insert(data[59999])
for address in trace:
    node = tree.node(address)
    mean = node.cover_mean()
    if mean is not None:
        print(f"\tNode {node.address()}, mean: {mean.mean()}")
    else:
        print(f"\tNode {node.address()}, MEAN IS BROKEN")

print("============= KL Divergence =============")
kl_tracker = tree.kl_div_sgd(0.005,0.8)
print("============= KL Divergence Normal Use =============")

for x in data[:50]:
    kl_tracker.push(x)

for kl,address in kl_tracker.all_kl():
    print(kl,address)

print("============= KL Divergence Attack =============")

kl_attack_tracker = tree.kl_div_sgd(0.005,0.8)
for i in range(50):
    kl_attack_tracker.push(data[0])

for kl,address in kl_attack_tracker.all_kl():
    print(kl,address)