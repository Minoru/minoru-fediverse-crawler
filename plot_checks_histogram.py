import matplotlib.pyplot as plt
import numpy as np

data = np.loadtxt('data.csv')
plt.hist(data, bins=150)
plt.show()
