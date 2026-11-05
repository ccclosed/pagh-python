class Animal:
    def __init__(self, name):
        self.name = name

    def speak(self):
        return self.name + " makes a sound"

    def __str__(self):
        return "Animal(" + self.name + ")"

class Dog(Animal):
    def speak(self):
        return self.name + " barks"

class Counter:
    def __init__(self):
        self.value = 0

    def bump(self):
        self.value = self.value + 1
        return self.value

animal = Animal("cat")
dog = Dog("Rex")
print(animal.speak())
print(dog.speak())
print(str(animal), "isinstance:", isinstance(dog, Animal), isinstance(animal, Dog))
counter = Counter()
print("counter:", counter.bump(), counter.bump(), counter.bump())
