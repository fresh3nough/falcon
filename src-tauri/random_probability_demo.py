# Random Probability Demo
import random

def demonstrate_probability(trials=1000):
    outcomes = {'Heads': 0, 'Tails': 0}
    for _ in range(trials):
        result = random.choice(['Heads', 'Tails'])
        outcomes[result] += 1
    probability_heads = outcomes['Heads'] / trials
    probability_tails = outcomes['Tails'] / trials
    print(f'After {trials} trials:')
    print(f'Heads: {outcomes["Heads"]} times, Probability: {probability_heads:.2f}')
    print(f'Tails: {outcomes["Tails"]} times, Probability: {probability_tails:.2f}')

if __name__ == '__main__':
    demonstrate_probability()
